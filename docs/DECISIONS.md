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
*Hard surfaces under the cluster LOD* (#309, 2026-10-19, found by a scene built outside the
repository): with geometry alone in the simplification error, a panel 1–2 cm proud of a face (a
drawer's front, a wardrobe's door) folds back onto the face behind it at about a pixel of error.
The two tie in depth and the jitter picks the winner each frame, a flash one frame in eight, and
the fold takes the panel's side normals (about 60 % too dark). Hard-surface pieces cook with
their normals weighed (`CookOptions::HARD_SURFACE`, 0.5, as the props' blocks and lathes): the
city plan's kit and the houses' insides since then; geometry alone stays for terrain, rocks and
foliage. Better still, model no face behind another (`forge_geom::check::stacked_faces` lists the
faces 1 mm to 3 cm in front of another facing the same way: the wardrobe's doors, the counter's
cupboard doors, the bed's panels). The village's views moved by at most 0.007
mean LDR-ꟻLIP from afar (the near ones not at all) for no measurable cost.
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
*Amended 2026-11-01 (#380, D-083's district):* the record holds a scale per axis: `scale` is the
largest axis's, `stretch` each axis's share of it (96 bytes again). An authored city stretches a
fifth of its instances along an axis (165,775 of the private district's 797,140, 30,107 distinct
mesh and scale pairs, axes up to 3 to 1 in the top tenth), which the uniform scale drew wrong.
Positions take the stretched frame everywhere (`instance_frame`, the TLAS's transforms), normals
its inverse transpose (`instance_normal_frame`), the bounds and the cut's error the largest axis
(conservative), and a stretched instance skips the normal-cone cull. A uniform scale keeps the
exact expressions it had (its stretch is exactly 1), so every scene before draws the same:
`add_instance_with_material` still keeps the x axis's scale for all three (city-plan's plants,
#380's open choice), `add_instance_scaled` (and the editor's `add_instance_stretched`) the three.
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

**Audio reads the row ✅ (Phase 7, 2026-10-23, `physics-lab`'s impacts).** A row's sound layer
(`SoundLayer`) names the surface its footsteps and impacts are made for, one of the foley's six
(stone, gravel, grass, wood, metal, water), or none for a material that rings of nothing (rubber,
cloth, flesh). The contacts each tick begins sound as their two bodies' rows: each rings, the
softer setting how long the contact lasts (`forge_audio::Impact::between`). The lab finds a body's
row by its prop, as its drawing does. Not yet: the absorption and transmission values (the
village's walls still name brick's in code), surfaces of their own for sand, mud, snow and ice,
and the deformable layer's sound.

**The deformable layer and Take-Two's US 11,534,688 B2** (to 2039, read 2026-10-24,
`research/impact-marks.md` §5.1). Its claim 1 stores the marks characters leave where the camera
is not, then writes them into a trail map round the camera when it comes near, with a shader by
terrain type. The layer's window (`Layer::move_to`) keeps only what lies where its old and new
places overlap, and gives the ground it takes on its plain depth: nothing pressed outside it is
kept. **It must stay so:** tracks kept far away would be a layer of their own, written at once
and drawn as they are, never stored to be replayed into a near map.

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
*The transport built* (#268, 2026-10-16, `forge-net`, `docs/demos/net.md`): QUIC through `quinn`,
TLS 1.3 from `rustls` on `ring`, certificates pinned by the client; the login tokens signed with
Ed25519 now rather than later, checked offline by the server from the issuer's public key; a hello
carrying the version, the channels and the token, refused with a code and a reason; reliable
ordered, reliable unordered and unreliable channels, unreliable messages fragmented past 1200
bytes. The link conditioner sits under quinn's socket rather than behind a `Transport` trait, so
the tests run the real stack on loopback. Measured: QUIC adds about 8 % to an 800-byte snapshot
at 30 Hz, so D-010's 24 KB/s a client takes snapshots of about 720 bytes with IPv4's headers.
*Replication built* (#268, the same day): the bit writer, cell-relative 1 cm positions,
29-bit smallest-three orientations, deltas against each entity's acknowledged state up to 31
packets back, interest by 32 m cells, the per-client priority accumulator under a 720-byte
budget. Two additions: entities are named by a per-client slot (a few bits a block), and a
client gets every entity's first state before any delta, which keeps clear of Verizon's
US 11,810,245 (`research/netcode.md`, §9; the owner to confirm). 100 clients over QUIC at
100 ms and 2 % loss took 23.6 kB/s each for 10 minutes on loopback, under the second proof
stage's 24 KB/s; authority handoff remains.
*Authority handoff built, in one process* (#268, 2026-10-23, `forge_sim::authority`): the layer
holds each entity's state, its owner and an epoch, and refuses a write at a stale epoch; it hands
an entity over at a tick's end once it is 2 m past its owner's strip, and workers take their
entities and a band of read-only ghosts from it at each tick's barrier, holding nothing between
ticks. 200 bots across a two-worker boundary step to the bit as one worker steps them, with
revokes arriving late (312 stale writes refused) and with a worker away and covered. The
protocol keeps clear of Improbable's 2016 family (`research/netcode.md` §9): no advance notice
of a handover, whole-entity authority taken as one picture a tick, one layer, one tick rate.
Over QUIC the same day (`forge_net::layer`): the layer on a server, each worker a client in
lockstep; a worker killed mid-tick is covered by another and the tick stepped again from the same
pictures, so the records still match one worker's to the bit.
*Pacing built* (2026-10-23, `forge_sim::PaceClock`): every input packet carries its client's
tick; the server reports the least lead on arrival of the last 60 in each snapshot, a measure
only; the client's clock runs within ±10 % of its rate from the lead's error alone. Over a route
that changes from 100 to 200 to 50 ms, the lead settles at its 3 ticks and no command is late once
settled. A late command still waits for the next tick, never repeated. Patents searched first
(`research/netcode.md` §9): none on the pacing; the acknowledged-only resends keep clear of Google's
US 9,479,618 B2.
*Corrections faded* (2026-10-23, the physics lab's `--net`): a corrected body stays drawn where
the prediction had it and the difference fades to 5 % in 0.1 s, by a fixed time; one more than
2 m off is drawn at once. At 100 ms one way the lab's corrections moved a body by at most 0.47 m.
*Replays built* (2026-10-23, `Server::record`, `Recording::seek`): the server records what it
applies, a digest every second and its whole state every 10 s; a replay starts at any tick from
the state before it and checks the digests on the way. The lab's networked session replays to
all its digests, whole or from part-way. Replays only reproduce: never play resumed from a
recorded moment (Blizzard's US 9,089,777 family, `research/netcode.md` §9).
*Regions by load, searched* (2026-10-23, `research/netcode.md` §9): Amazon's US 12,681,773 (to 2044)
claims moving an identified partition to another worker on worker-level resource measures, and
HCL's and Sony's families claim moving a boundary by load and adding servers per area. The rules
kept: a fixed worker pool, strip *i* always worker *i*, the cuts moved by the layer alone from
entity positions, no per-worker load in a decision. Re-cutting on a worker joining or leaving
is built (`Strips::balanced`, `LayerHost::recut`: the records still the one-worker run's to the
bit); moving the cuts during play waits for the owner's word.

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
**Imported rigs and root motion** (#310, #311, 2026-10-19, needs found by a character test built
outside the repository): a clip's channels on nodes between a joint and its parent (or above a
root) that are not joints are baked into the joint under them at 60 keys a second, so a rig whose
root frame above the pelvis carries the travel and turn moves (the skeleton and the vertices'
joint indices unchanged). `Clip::root_motion` gives the root joint's move and turn about +Y
between two times, in the character's frame at the first (over a loop's end too), for a
controller to apply, and `lock_root` holds the pose's root at an anchor while it does: the whole
motion applied and locked reproduces the clip's own path, the hips' sway included. The turn is
the rotation's twist about +Y, a quaternion with `sqrt` only (D-016). **Layers** (#313):
`Pose::mix_masked` mixes a clip over another on a joint mask (`Skeleton::mask_below`: a joint and
what it carries), and `Pose::add` adds an additive layer's difference from its reference, its
rotation in the joint's own frame as engines' additive layers do.
**A state machine as data** (#312, 2026-10-19, in the shape an out-of-tree prototype settled on
for a player of about 125 nodes and a pursuer of about 40, `forge_anim::machine`):
- **Action states** with their ways out, opened by the game's variables and posted events. The
  most specific way goes first, then the written order. Several steps run in an update, never
  back to a state already passed in it: without that a variable the game sets and the host does
  not bounced between two states every frame.
- **Animation nodes** in each state: the one whose conditions hold with the most conditions
  plays, then the highest priority, then the first (priority first let a three-condition sprint
  vault lose to a one-condition start). A node may extend another (a state's or a base): the
  parent's fields, then the child's, its lists appended, so a variant is a few lines.
- **Switches** go through `Player::play` (inertialized). A transition names its blend and an
  optional in-between clip played once first. A one-shot holds its end.
- **Events** at a node's start, its end or a share of it go to the host. `SetVariable Name=value`
  also sets the variable, which sequences a vault's start, its end and the way back.
- **A looped node** can play at the character's speed over its clip's natural speed (0.5–1.8×).
- **Variables** are named without case; a missing one reads false, 0 or "". Events the game posts
  are seen by the next update, then cleared.
- **Built** from TOML (`MachineDef::from_toml`) or in code, bound to a rig's clips by name
  (`Machine::new`), run per character (`Animator`, with `describe` for a live view).
- **After its first user's run** (the same day: a player of 13 nodes and a pursuer of 38 on its
  rigs, 598/600 and 480/480 frames as its own interpreter drew them):
  - **Gaits:** `match_speed` applies to blends too. The natural speed is the blend's root speed as
    it plays (its clips' travel over a cycle mixed by their weights, times their cycles a second
    mixed the same way), so a run at 3.4 m/s no longer cycled 1.7 times too fast. The matched
    rate replaces the node's own.
  - **In-between clips:** a transition's in-between clip plays at its own rate
    (`TransitionDef::speed`).
  - **Missing clips:** `Machine::new_dropping_missing` leaves out what names a clip the rig lacks
    and lists it (the data names hundreds of clips, a rig carries a few dozen).
  - **Root motion:** `Animator::root_motion` gives the root's motion over the last update, through
    blends (the clips' motions mixed, `RootMotion::mix`) and in-between clips.
  - **Smaller fixes:** numbers compare to 10⁻⁴; posted events are kept once each, at most 32.
- **Layers over the states** (#329, 2026-10-21, a game's rifle: aiming while walking):
  `[[layer]]` names a joint (its subtree is the layer's mask), a mode (`override` or `additive`,
  the latter from its node's first frame), a variable scaling its weight, a fade (0.2 s) and
  conditions. Its nodes are chosen as a state's, a blend by the aim's pitch among them. Its weight
  fades in while its conditions hold and out when they stop. The root's motion stays the states'.
  Tests on a four-joint body: the legs and the root exactly the walk's under an aiming upper body,
  half the fade halfway, a recoil added from its first frame.
- **Left:** typed event channels in place of strings, and the overlay's or the console's live view
  (the prototype's asks).
- **Patents:** none read on a machine loaded from local data. Zynga's US 9,463,386 B1 and
  US 9,821,230 B2 (D-072) claim a definition received from a server: reread them before a server
  sends a machine to clients.

**Hit reactions: the strength schedule ✅ (Phase 7, 2026-10-24, `physics-lab --lab creatures`).**
The physics layer's "hits simulated rather than played":
- **A blow:** a moving body striking a creature's part at 0.5 m/s or more (not the ground, its stand
  or its own parts), from the contacts each step begins. Its impulse `μ v` (μ the reduced mass of
  the part and what struck it) lowers the part's strength; 8 N·s takes it to 10 %, never to none.
  Its parent and children lose half as much.
- **Strength:** scales the part's joint's motors, their stiffness, damping and torque
  (`World::drive_ragdoll_with`). Each part recovers to full over 0.6 s while its clip plays on,
  so the body is knocked off its pose and pulled back.
- **Simulation state:** saved, restored and replayed with the world. A ball thrown at a mannequin:
  2 blows, its weakest part at 0.10, all back to full two seconds on; a run restored mid-recovery
  keeps the unbroken run's digests (`a_blow_weakens_a_creatures_parts_and_they_recover`).
- **Patents** (`research/hit-reactions.md`): the whole technique, with the limp fall and the
  centre-of-mass balance check, is in EA's US 7,403,202 B1 (filed 2005, expired). The live claims
  read need what this does not do, and must keep not doing:
  - a clip, frame or start frame chosen by matching against the hit or the ragdoll's pose (EA's
    US 9,827,496 and US 10,388,053; a get-up must not start at the clip frame nearest the pose);
  - a hit clip chosen by the collision and warped (EA's US 10,022,628);
  - EMG-recorded muscle data (EA's US 10,497,163);
  - a struck limb made heavier or longer (Sony's US 11,113,862).
- **The fall and the rise** (the same day): a dog's balance takes its torso's strength, so a blow
  there staggers it. A blow of 20 N·s or more, or its torso tilting past 50° (the balance check),
  fells it. It lies slack at 2 % for 1.5 s, then rises over 2 s as its strength and balance come
  back, its balance unchecked until it is up (a hard blow still fells it as it rises; let go with
  ↓ it lies as it is, no fall counted). A thrown ball fells a dog 5 ticks after leaving the
  hand (its torso 0.57 → 0.20 m); four seconds on it stands at 0.56 m and walks its round
  (`a_dog_knocked_down_falls_lies_and_gets_up`). The rise is the dog's balance spring pulling it
  up, the lab's stand-in: a get-up clip would start from its first frame, never from the frame
  nearest the ragdoll's pose (EA's US 9,827,496).
- **Shots** (the same day, `LabCommand::Shoot`, `physics-lab --shoot-every N`): a ray of 100 m;
  the body it meets is pushed by a rifle bullet's 8 N·s at the point met, and a creature's part
  takes a blow of it (a ray makes no contact for the reactions to find). One shot at a dog is a
  full blow on the part it meets and does not fell it (`a_shot_strikes_the_part_it_meets`).
- **Next:** the shot rewound to the moment its shooter saw (#268's lag compensation in a scene),
  a mannequin's fall from its stand.

**Amended (2026-11-07): a skinned mesh whose rays keep it as cooked** (the owner's go: "go with the skinned rays option").
`MeshletSceneBuilder::set_skin_rays(mesh, false)`: the skin pass moves what is drawn, but the ray-traced copy keeps the mesh's
vertices as cooked. Its structure is a static one, never refitted, and the pass writes none of its ray positions. The rays still
follow by default, so every scene so far renders exactly as before.

- **Why:** refitting every skinned mesh's structure every frame costs as many structures as there are meshes. A crowd of 1,017
  skinned meshes (6 million triangles) took about 1.7 ms a frame in refits on top of 0.9 ms of skinning, at 2560 x 1440.
- **The trade:** shadows, reflections and bounce light keep the cooked pose. A body that barely moves, cooked in the pose it holds
  most (a crowd's idle loops), loses little. A body cooked in a pose it leaves, such as a rig's bind pose, casts that pose's
  shadow, so the option is for the first kind.
- **Where the copy stands:** where the mover alone places it. Whatever the joints' matrices carry (a heading, a root's offset)
  is lost, so it suits a mesh whose joints move it about its mover's frame only. The lab's creatures and gulls are the other kind:
  their placement is in their joints, so under the lab's flag their copies stand unturned and too high, and their shadows are
  wrong. The lab's run measures the cost, not the look. A still copy's cut also takes a static mesh's steps: its cut-out and jelly
  rows split out and opacity micromaps, which moves a jelly slime's shadow-ray start a little under the flag.
- **Measured, cost only,** on `physics-lab --lab creatures --skin-rays-still` (its mannequins, dogs and slimes, 9 skinned meshes), 1920 x
  1080, 900 frames, two alternated runs each: the frame 5.17 and 5.16 ms with the rays following, 4.94 and 4.95 ms with them still.
  `skin/blas` (0.28 to 0.29 ms) is gone; `movers/tlas` reads 0.27 ms instead of 0.13, because the refits had hidden part of its
  zone: the profiler's zones run from mark to mark, and a pass beside another reads shorter. The frame is the measure.

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

- **Foliage under the cluster LOD** (#303, found by a scene built outside the repository; the
  ladder's far end is D-075):
  - **Leaf cards and open blade strips never simplify.** The simplifier keeps a mesh's open
    borders (`LockBorder` in `forge_geom::lod`), and a card is all border, so a far crown draws
    and traces every card. Freeing the borders thins a crown unless the remaining leaves grow,
    and that is what Pixar's US 7,973,789 B2 claims (to about 2028-11). D-075 keeps the counts
    fixed until then: a far crown hands over to closed shells and impostors instead.
  - **Two sheets at the same positions collapse.** The simplifier tells a seam from a border by
    position, so a two-sided surface made of a front and a back sheet on the same vertices
    reads as closed. It went at 1–2 cm of error, the blades vanishing a few metres away,
    whatever the normal weight. Keep the back sheet off the front by 1.5 mm or more, as
    `forge_geom::modules::sheet` does (2 cm), or draw one sheet with a double-sided row.

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

**Amendment ✅ (2026-10-10, the owner's go on D-058's step 2): a generator on the GPU may be
authoritative, in integers.**
- **What may decide:** what the CPU, the GPU and a server must agree on (the ground's heights, a
  physics height field) is made in integers only. That means fixed-point arithmetic on 64-bit
  integers, with the same code in Rust and in Slang. Every operation is exact by both languages'
  definitions: wrapping add and multiply, shifts, integer division.
- **Floats on the GPU only shade** (D-058's normal maps). Vulkan's precision rules
  (`docs.vulkan.org`, "Precision and Operation of SPIR-V Instructions") don't make them the same
  bits everywhere:
  - 32-bit add, subtract and multiply are correctly rounded, but in the implementation's rounding
    mode unless the entry point declares `RoundingModeRTE`;
  - denormals "may be flushed to zero" unless `DenormPreserve` is declared, which not every device
    supports;
  - a multiply and an add may fuse unless decorated `NoContraction`, and the decoration does not
    reach an extended instruction such as `FMix` (Slang's `lerp`: #281, `shaders/exact.slang`).
    Since #282 (2026-10-19) a precise entry's SPIR-V has each `FMix(a, b, t)` written out as
    `(b − a) · t + a`, each operation `NoContraction`, whatever its source says
    (`forge-gpu/src/exact_mix.rs`, cached as the `O2px` variant). The images moved by rounding:
    1 to 4 pixels of a sentinel, by at most 4 levels;
  - for doubles, "the precision of double-precision instructions is at least that of single
    precision", and nothing more.
- **One definition:** the CPU runs the same integer code, as the reference, the fallback and the
  server's. It is not a float original that the GPU approximates.
- **Checked by digests:** each authoritative GPU generator comes with a test that runs its kernel
  on the device and compares its bits with the CPU's. It runs on the RTX 5070 Ti and on the AMD
  iGPU (`FORGE_GPU=amd`).

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
default and the fallback** (until D-070, 2026-10-18: DLSS is the default and the captures'
image, and FSR 3.1 replaces TAA); U switches TAA → DLAA → Quality → Balanced → Performance →
Ultra Performance at run time (`--upscaler` at start), and the profiler zones are
`temporal/TAA resolve` or `temporal/DLSS` + `post/display transform`.

DLSS reads the same inputs as the TAA resolve: the jittered pre-exposed HDR colour, the
reversed-Z depth, the motion vectors (UV offsets, unjittered) and a 1 × 1 exposure of 1.
Pre-exposure is passed relative to EV100 15 (settled, at most 1 since D-070's amendment of 2026-11-08);
DLSS's output kept the input's level. The
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
IMMEDIATE when Streamline is loaded. *Amended 2026-10-11:* only in scripted runs, whose frame
times are measured (`Pacing::Uncapped`). Interactive runs never tear (`Pacing::Smooth`: MAILBOX,
else FIFO). At hundreds of frames a second, IMMEDIATE tore every interactive DLAA run into
bands drawn at different times as the camera moved. The owner saw building faces drawn "late",
"like a v-sync issue".

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
- **Amended (2026-10-30, the owner: "we should disable the 40,000 triangles limit, just have
  the mesh simplifier warn"):** a mesh of up to 250,000 triangles is traced as drawn, every
  triangle (`raytrace::FULL_DETAIL_UP_TO`). Over it, the mesh is cut to 40,000 as before and the
  scene's load warns, naming the five largest; the terrain keeps its 600,000.
  - **Why:** the Bistro's large tree, 45,540 triangles, was cut, and the rays from its drawn
    bark started inside the traced trunk, met its back and found no light: the trunk came out
    black in every mode (#367).
  - **Why not every mesh:** the city's 21 props are 33.8 M triangles drawn in full (its towers 3
    to 8 M each) and the island's 94 are 143.5 M. Traced whole, they would take gigabytes. The
    rays' starts by the cut make those safe instead (#367: the shadow rays' already, the
    bounce's in progress).
  - **Measured** (three runs each, alternating): the village 1.48 M → 1.56 M traced triangles,
    119 → 124 MiB, its structures built in 158 → 162 ms, the frame the same (4.83 → 4.86 ms). The
    island 2.36 M → 3.39 M, 157 → 218 MiB, built in about 0.7 → 1.3 s, the frame the same. The
    city is unchanged: every one of its props is over the limit.

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

- **What sees an instance** (#301, 2026-10-19; `Visibility`, `set_instance_visibility`).
  - **The need:** an instance drawn but not traced, or traced but not drawn: #297's crown
    proxies, which only the probes' rays meet, in place of the leaf cards. Before, the only
    traced-but-not-drawn instances were the `drawn_after` rows', by accident.
  - **How:** four bits of the instance's flags (`INSTANCE_NO_RASTER`, `INSTANCE_NO_SHADOW_RAYS`,
    `INSTANCE_NO_PROBE_RAYS`, `INSTANCE_NO_MIRROR_RAYS`). The culls skip an instance that is not
    drawn. The TLAS build turns the others into the instance's 8-bit mask (`RAY_SHADOW` 1,
    `RAY_PROBE` 2, `RAY_MIRROR` 4), and each pass's rays carry their own (`g_ray_mask`): shadow
    rays by default, the probes' pass its own for its rays and their hits' shadow rays, the
    reflections' and the requested rays' theirs. The particles', the dust's and the splashes'
    rays are shadow rays. A dynamic scene sets it through `SceneEditor::set_instance_visibility`.
  - **Checks:** with every instance seen by everything, the captures are those of the previous
    build.
- **A cut-out row's triangles alone reach the query loop** (#307, 2026-10-19; D-075's first
  point).
  - **Before:** an instance with a cut-out or jelly row was non-opaque as a whole
    (`FORCE_NO_OPAQUE`), so a ray through a tree tested its wood and its crown's cores in the
    candidate loop as well as its cards.
  - **Now:** the scene build finds the sections each mesh's instances draw cut out or as jelly
    (`Cut::split`). Their triangles go last in the cut, and the mesh's structure takes two
    geometries: the others opaque, those not (`BlasTriangles::opaque`). A ray's triangle `p` in
    the second geometry is the cut's `opaque_triangles + p` (`cut_triangle`).
    - An instance whose rows are all opaque is forced opaque.
    - One whose cut-out rows fall in the second geometry takes the geometries' own opacity.
    - Any other (a dynamic scene's meshes, the skinned ones, a mover given a cut-out row after
      the build) stays non-opaque as a whole.
  - **Measured** (the village, best of three, against the build before):

    | View | frame | `gi/probe rays` | `shadow/trace` |
    |---|---|---|---|
    | the main street, 15:00 | 4.63 → 4.57 ms | 1.49 → 1.38 | 0.340 → 0.344 |
    | the green | 4.24 → 4.19 ms | 1.22 → 1.15 | 0.131 → 0.136 |
    | the garden | 3.63 → 3.61 ms | 0.81 → 0.78 | 0.100 → 0.103 |
    | the junction at 21:00 | 4.93 → 4.91 ms | 1.24 → 1.18 | 0.218 → 0.222 |

    - **The probes gain most:** since #297 they meet no leaf card, and the kit's gratings' frames
      no longer enter the loop.
    - **The sun's and the lamps' rays lose 1–3 %** (the lamps' rays 0.308 → 0.320 ms at the
      junction). Two geometries in one structure trace a little slower than one, and the
      village's trees are mostly cards.
  - **Checks:** the map check's 12 views 0 px; the day check's sentinels 0 px; the lab's models,
    materials, slime, room and tank, the city's culled and orbit views and the island 0 px. Sponza
    is the exception, 2 px by 3 levels: a probe keeps a quarter of the cut-out triangles it meets
    by a hash of the triangle's index (`probe_leaf_kept`), and the cut-out triangles moved to the
    end of the cut.

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
  - **A share by roughness** (#306, 2026-10-19; `mirror_ray_share`): the ray traces all of
    the mirror reflection up to roughness 0.30 (Blinn-Phong about 245: glass at 3000, water at
    900, a vehicle's at 300), fading to none at 0.42 (the exponent 60). The rest takes the
    probes' dimmed sky, as an untraced row's does (#68).
  - **Why:** at the exponent 60 a row went at once from no ray to 70 % of a sharp mirror. A
    glossy tile floor indoors (90, roughness 0.38) showed its room point-sampled at grazing
    angles, a hatch of blue-white streaks. The reflections' history held it steady: without the
    history, or without the rays, it was gone. A scene built outside the repository and the
    village with such a floor showed it alike.
  - **Now:** that floor traces 21 % of its mirror. The streaks are faint, and a still camera's
    frames 10 apart differ on 10 600 pixels rather than 42 000. What is left lies along the
    wardrobe's foot, where rays slip under it. The city's start view changes on 1 760 pixels
    (far lamp posts); the glass and the water are unchanged.
- **The hit data:** the cuts stay on the GPU after the BLAS builds, 34 MiB with each
  triangle's section. A hit reads its instance from the record's custom index and its
  triangle from the cut.
- **The hit's light:** the row's colour times its texture's average, the sun through a
  shadow ray, the sky's SH.
- **The start:** hits count from 1.5 m, past the cut's error. Since #305 (2026-10-19), a drawn
  surface's mirror ray (`shading/reflections`) starts at twice the gap the traced cut may leave
  in front of it: the cut's error and the drawn cluster's own, scaled, at least 2 cm and at most
  1.5 m (`mirror_start`, as #293's lamp rays). From 1.5 m everywhere, glossy furniture and a
  tiled floor indoors reflected the street past their room's walls, a parked car included
  (seen in a scene built outside the repository). The water's, the glass's and the jelly's rays
  keep 1.5 m: they start on surfaces the rays' scene does not hold. The city's towers, cut
  coarsely, keep nearly the old start.

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
- Mod code, if wanted: Wasm components in wasmtime. (D-072, 2026-10-18: Luau for mod code
  outside the predicted step; Wasm for mod code inside it.)
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
  - **The distance map** weighs the rays by the cosine to the power 50 around each texel's
    direction, so it keeps the shape of what surrounds the probe.
  - **In room-sized cascades** (1.5 m apart at most: the village's 1 m, the models lab's rooms)
    the power is 10 (#283, 2026-10-18). There a probe stands a few tens of centimetres from a
    ceiling, a reveal or a stair's opening and sees its edge at a grazing angle. The test's
    boundary then took the map's 14-texel grid: a sawtooth shadow on the village hall's ceiling,
    by day and at night.
  - **What the wider lobe does:** the boundary becomes a soft gradient. The village's views
    changed by at most 0.012 EV, except the landing above the stair, 0.21 EV lighter and more even.
    Coarser cascades keep 50, to the bit.
  - **Tried and left aside:** probes kept farther from surfaces or a larger bias (the teeth
    shrank), 30-texel maps (finer teeth), and four taps at the lookup. Averaging the taps'
    visibility drew a kinked shadow on an open window leaf. Averaging their distances left
    faint teeth, at four times the fetches.

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

- **The probes do not see the movers by default** (#299, 2026-10-18; `ProbeParams::movers`).
  - **Before:** the probes' rays met the movers (#79), and the probes the movers entered
    settled again (#69). The probes' passes, on the async compute queue, read this frame's
    movers and so waited for their upload on the graphics queue, which itself waited for the
    previous frame's probes (#198 chose that wait: on the island it kept the probes off the
    culls).
  - **What it cost:** the village's main street with 23 walkers took 7.18 ms against 6.53 ms
    without them, `gi/probe rays` 5.08 against 3.32 ms. The serial frame was the same with or
    without them (6.6 ms), and the probes skipping the movers' ray query alone changed nothing:
    the cost was the wait, not the rays. A scene built outside the repository saw the same
    doubling with 150 or 17 000 movers, near or far.
  - **Now:** off by default, the probes neither declare the movers nor wake on them, and their
    rays skip the movers' structure (`g_trace_movers`, set by `probe_trace_main`). The movers
    keep their sun and lamp shadows, their reflections and the other passes' rays; only the
    probes' diffuse light does not see them. A scene whose movers are what it shows turns it on
    (`Look::probes_see_movers`): the physics lab's scenes do. Without its bodies, 40 of the
    lab's captures changed by up to 5 % of their pixels, light leaking round the crates, the
    dominoes and the drop's blocks. The city's and the island's captures did not change.
  - **Measured** (best of three, 240 frames): with the walkers 6.51–6.53 ms, without them
    6.49–6.50 ms; `gi/probe rays` 3.28–3.30 ms with them.
- **The probes meet a plant's crown proxy, not its leaf cards** (#297, 2026-10-19; city-plan's
  forest, through #301's `Visibility`).
  - **Before:** a probe ray crossing a crown tested each card it met against the leaf atlas, a
    candidate each in the query's loop. Under the village's holm oaks the probes' rays took
    3.0 ms of a 6.2 ms frame.
  - **Now:** each grown plant is cooked with a proxy (`grown_proxy`): its wood, and each lump of
    its crown a closed shell of 0.6 of the lump's radius, opaque, in the leaves' colour. The
    proxy instance is seen by the probes' rays alone (`Visibility::PROBES`), the plant by
    everything else. `--leaf-probes` is the A/B.
  - **The share, by the light** (15:00, a fixed exposure, against the probes through the cards):
    under the main street's holm oaks −0.15 EV and ꟻLIP 0.074 at 0.6, −0.32 EV and 0.099 at 0.8,
    +0.04 EV and 0.080 at 0.5; the air view, the garden and the green within +0.07 EV. Without the
    plants in the probes at all, the street's shade was 2.45 EV brighter and the crowns glowed.
  - **Measured** (best of three, 240 frames):

    | View | cards: frame, `gi/probe rays` | proxy | no plants |
    |---|---|---|---|
    | the main street | 6.16, 3.02 ms | 4.63–4.66, 1.48–1.49 ms | 3.75, 0.79 ms |
    | the green | 5.43, 2.43 ms | 4.24, 1.22 ms | 3.57, 0.70 ms |
    | the garden | 4.34, 1.54 ms | 3.62, 0.82 ms | 3.34, 0.59 ms |
    | the air view | 4.41, 1.45 ms | 4.39, 1.32 ms | 4.28, 1.28 ms |

    The proxy's share barely moves the cost (the main street 4.63–4.66 ms from 0.6 to 0.8).

- **The frame after a reset updates every probe eight times** (#320, 2026-10-19; found by an
  out-of-tree scene: rooms seen through windows black as it opened). The first round lights its
  rays' hits by the sun alone (#171), each next one by the sun and the maps the round before
  left, every probe new in each (its maps replaced whole).
  - **Measured** (the village's room and hall at a fixed exposure, the first frame against the
    twentieth): 15 % of the light with one round, 62 % with three, 85 % with six, 91 % with
    eight.
  - **The cost:** about 1.4 ms a round, on that frame alone (the start, a jump of the camera).
  - **Every probe scene's history moves with it:** at frame 20 the room stood 16 % brighter than
    after a single round, so the captures of the scenes with probes were accepted again.

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
proposed ("go with recommendations"). *Amended by D-059 (2026-10-11):* the first style set is a
Mediterranean-European perimeter block for the city sandbox, ahead of the medieval village.

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

*For a while (D-059, 10; the owner, 2026-10-10):* the city sandbox's checks and its "day in the
city" flight are the regression check. The gate still runs on every push, and the demos'
captures run when a change touches what they draw.

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
  - **Changed 2026-10-10 (#220):** shown the planet under AgX and under its punchy look a stop
    brighter, the owner chose the second ("the one on the right is way better"), then asked for
    it in the other demos too. Every demo but the ballad (ACES, for space's black) draws with
    AgX's punchy look, the fixed exposure at EV 14 and the metered one a stop up; the punchy
    curve alone darkened the mid-tones. That settles the exposure's stop below.
- **Still open:**
  - bloom's strength;
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
  differ by up to 3 codes from run to run, and the captures' checks want them to the bit.
  (D-070, 2026-10-18: that difference was Forge's shading, made deterministic by #277 and
  #281; DLAA's images are now equal run to run, and the captures move to DLAA.) Its
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
- **Emissive light (2026-10-15, #254):** the shading stays per unit of the frame's reference
  illuminance. A row may now give its emission in cd/m² (`RenderLayer::emissive_absolute`), and
  the shader converts it each frame by that reference (L·π / E). Lamps and lit windows then hold
  through a running day, where a value in the shading's units is worth 160 cd/m² by day and
  almost nothing at night. The rays' hits emit as the frame shows them (an emissive map's
  average, a window's room at its lamp's share), so the probes no longer flood the night.

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

**Block compression, first used (#302, 2026-10-22):** by the impostors' atlases (D-075 step 4),
encoded on the GPU at their bake by an encoder of our own (`shaders/impostor_compress.slang`:
BC7 mode 6 and BC5), so no encoder crate came in. `textureCompressionBC` is now a required
device feature (every target GPU has it). The models' and materials' textures are still RGBA8.

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
  - **Since #210 (2026-10-09): NRD 4.18.0, `master` at d3df343**, then its tag (8f12a22,
    released 2026-10-30, nine commits on: REBLUR's temporal accumulation normalises its
    neighbours' normals safely, where opposing ones cancelled to a NaN; the rest is the SH and
    SG resolves and RELAX, which Forge doesn't use; the headers unchanged).
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

**In motion over the water** (the owner, 2026-10-28: the trees' and the clouds' reflections smear
on the village's water when strafing, gone with `r.reflections.rays 0`):
- **The history kept the wrong quantity.** It held what the ray added: the hit's light minus the
  sky the pixel was shaded with in that frame. The resolve draws that sky afresh each frame, and
  the sky, at infinity, slides over a mirror unlike the trees it reflects, so a difference against
  one frame's sky came back under another's. Moving sideways at 60 m/s, a cloud showed through
  the trees' reflection as a pale streak and a blue band opened under the quay (the history off,
  neither). It now keeps the light the mirror direction showed (what the ray met, or the sky as
  the frame shows it) and takes the frame's own sky off after the blend; held still, the same.
- **The upscaler followed the surface.** DLSS reprojects its history by the motion vectors, the
  water's own, while a reflection moves as its virtual point does (slower across the screen,
  sideways, than the water nearer the eye). The mirror rays' pass now moves each pixel's motion
  vector towards its reflection's virtual point (the one the history reprojects; the sky's at
  infinity on a miss), by twice the reflection's weight, capped at all of it; a mover's
  reflection keeps the surface's. `r.reflections.motion` (on by default; the owner, after
  trying it: "it's better with it on, keep it"). The pass takes the motion vectors' index in
  the push's ambient occlusion slot, which it never reads: the push is at 128 bytes. 0.003 ms.
- **Tried and left out:** a reactive mask from the reflection's share of the pixel. FSR 3.1 reads
  it; DLSS (its default preset) ignores the bias hint, the frames to the pixel the same, as with
  the spray's and the particles' masks (the audit's step 1).
- **A curved mirror (2026-11-08, the owner: "a trail from the bottles when the camera is
  moving").** The virtual point stood as far behind the surface as the ray had gone, which only
  a flat mirror does. A convex one images what it reflects close behind itself: by the mirror
  equation 1/image = 1/distance + 2 × curvature, the sky at half the radius. In the private
  district a bottle (a smooth metal row a metre from the camera, passing at about 35 pixels a
  frame) took the motion of rooms metres away, and the upscalers tore it and left it behind.
  `mirror_image_distance` now gives that distance, the curvature being the largest turn of the
  normal along the triangle's edges (`triangle_curvature`), for the motion vector and the
  history alike. A triangle whose corners' normals agree (curvature 0) keeps the old
  arithmetic, so still water and panes do not move: with the anti-aliasing off, the private
  district's frame changed only on curved shiny objects. A concave mirror is taken as a convex
  one of the same curvature, an approximation. The other half of that trail was DLSS's
  pre-exposure (D-070's amendment of the same day).
- **Patents** (`docs/research/glass-credibility.md`, "Patents"): Autodesk's US 9,355,475 (to
  ~2035) claims a surface motion vector, a second one for the reflected object and a projection
  onto the surface's previous position; this moves one virtual point by the camera's motion over
  a still surface, leaving movers out. NVIDIA's US 11,836,844 is about refractive interfaces.
  The owner chose to build it after reading the summary. The curved case only shortens that one
  point's distance: the reading stands.

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

**Step 5's first part (2026-11-07, #214; the owner: "move the effort to ... the content package and cell streaming"):**
`forge_core::package`, the format and its reader and writer.

- **The directory:** `manifest.toml` plus the entries' files. The manifest lists each entry's id, kind, format version, file, size and the
  xxh3 digest of its whole file.
- **Two kinds of entry:**
  - a `Stored` value, written as a short header and the payload packed in the derived-data cache's frames;
  - a file copied as it is.
- **What a read refuses,** naming the entry: another kind or format, a size or digest that isn't the manifest's, a missing entry, and a
  manifest of another format. Unit tests cover each.

Next is the cook and the runtime's use of it, on the private district.
- **Why:** its load takes 24 s on one thread and uploads 8.6 GiB.
- **Where the time goes:**
  - the layered materials' setups, 7.5 s;
  - the scene build, 4.9 s;
  - the maps, 2.7 s;
  - the rays' structures, 1.9 s.
- **What the package holds first:**
  - the setups' derived data;
  - its cells' records;
  - its textures, in the format D-105 settles.

**Step 5's second part (2026-11-07): cooked meshes in a package.** The meshes came first: they are what a game can't
make without the source's models, and a picture checks them exactly.

- **The engine:**
  - `forge_geom::cache::package_mesh` copies a cached mesh's file in as it is: kind `mesh`, its format the cache file's
    and the cook's versions together, so a package cooked by other code is refused.
  - `read_packaged` reads it back without the parameters its key was made from, in one of two ways:
    - whole, its digest checked;
    - as its hierarchy alone, its pages left in the package's file for the page streamer. `Package::file` checks the
      size; the digest waits for a whole read.
- **On the private district:** its viewer writes its 14,930 drawn meshes into a package (953 MiB, 6.8 s) and draws from
  it with no mesh cooked and no model looked up. Read back:

  | | From the cache | From the package |
  |---|---|---|
  | Resident, whole | 0.56 to 0.64 s | 0.46 s |
  | Streamed, the hierarchies alone | 0.21 s | 0.12 s |

- **The checks:**
  - **The look:** four of the baseline's stills from the package differ from the cache's as the cache's differ from
    themselves run to run (121, 155, 234 and 0 pixels against 115, 158, 271 and 0).
  - **The streamed path, real time:** 15.94 ms a frame against 15.81 from the cache. VRAM and geometry are the same.
  - **A corrupted entry:** one byte changed in a mesh entry refuses the package, naming the entry.
  - **Unit tests:** both readings, and both refusals.

**Then the layered setups' masks, stored as made (the same day).** The private district's viewer keeps each setup's mask
textures and presence map as `layer_mask_textures` and `layer_presence` make them. It stores them as a `Stored` value in
two places:
- **the derived-data cache:** keyed by the mask files (path, size, time), the options that shape the masks, and the
  viewer's binary. Setups that share their mask files share one entry.
- **the package:** written with the meshes, one entry a setup.

| The district's 2,424 setups | Made at each start | Read as made |
|---|---|---|
| Their masks made or read | 2.6 s (3.9 s the first time, with the stores) | 0.70 s from the cache, 0.72 to 0.76 s from the package |
| The whole step, with the upload | 4.6 s | 2.06 to 2.14 s |
| The streamed path's loading screen | 7.3 s | 6.2 s from either |

- **Size:** 480 MiB in the cache and 686 MiB in the package, where shared setups are stored once each.
- **The look:** the stills are within the runs' noise, read from the cache or the package against made afresh (127 to
  240 pixels of 3.7 million).
- **What is left of the step:** the masks' upload, 1.3 s. Their rows are RGBA8 with full mip chains; a block-compressed
  layout is D-105's to settle.

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
  own. At Èze the fog was mostly AgX's greyish sky: shown AgX's punchy look at EV 14 beside it,
  the owner chose it ("the one on the right is way better"), the planet's default since, and
  every other demo's but the ballad's (D-045).
- **Faster new tiles** (2026-10-10, `docs/demos/planet.md`, "A new tile"): a fine tile waits
  0.09 s rather than 0.46 (0.27 s of one core; its heights and its DAG's groups spread over the
  threads); the caches' stores no longer scan their directories; the normal map's unread samples
  left out and the noise's seeds kept (the same bits); the DAG's first level cut into the grid's
  7 × 7 cells (the same errors and GPU frame, other clusters). A DAG template shared by every
  tile cooked in 23 ms but erred 7–10 times as much: set aside. Then (evening) 0.055 s, 0.18 s of
  one core: the DAG's clusters' bounds made on the threads with the clusters (the same bits),
  the whole map's noise left out where a region replaces it, the noise's corners hashed side by
  side. The first start's 378 tiles cook in 10.9 s rather than 14–16.
- **Next:** a tile in milliseconds still (its DAG 35 ms waited, 0.11 s of one core; its normal map
  about 50 ms of one core): proposed as D-058 🟡, the normal maps and heights on the GPU and the
  DAG measured.

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

## D-058 — A planet's tiles made on the GPU: the normal maps first, the heights under D-016's GPU rule, the DAG measured ✅ (proposed 2026-10-10, #220; steps 1 and 2 built, step 3 measured the same day: the DAG stays)

D-057 is kept for #222's smart systems.

**The question:** the owner asked for "tiles cooked in milliseconds" (2026-10-10). The CPU work
has gone as far as it cheaply goes (`docs/demos/planet.md`, "A new tile"):
- a new fine tile waits about 55 ms (0.46 s that morning), 0.18 s of one core;
- its DAG takes 35 ms waited, about 0.14 s of CPU: meshoptimizer's simplifying and clustering,
  each level's partition on one thread;
- its normal map takes 9 ms waited, about 50 ms of one core: 197 000 heights;
- its heights take 3.6 ms waited, about 16 ms: 67 000 heights at 0.25–0.33 µs, mostly the noise;
- storing takes 6 ms.

Two cheaper ideas were measured and set aside the same day:
- the children taking their parent's samples, exact to the bit: only 1–13 % of the tour's tiles
  found them;
- the cooking threads below normal priority: the frames got worse, and the frame thread above
  normal was kept instead.

**What the research says** (`docs/research/planet-terrain.md`, `worlds-at-every-scale.md` §9 B):
- **The engines that make a planet as it is flown generate its terrain on the GPU:**
  - SpaceEngine writes elevation in a shader and reads it back, and reported the move at 100–200×
    the CPU;
  - Elite's compute shaders evaluate the noise per patch, with 64-bit and emulated dual-float
    libraries for its coordinates;
  - Outerra refines 90 m data per tile on the GPU.
- **Forge keeps its authoritative data on the CPU (D-016),** so the server's collision heights are
  the client's bytes.
- **What it takes for the GPU to give the same bits on every vendor:** Vulkan's 32-bit add,
  subtract and multiply are correctly rounded, but:
  - the rounding mode is the implementation's unless the entry point declares `RoundingModeRTE`;
  - denormals may be flushed;
  - a multiply and an add may fuse unless decorated `NoContraction`;
  - division is only within 2.5 ULP.

  So it takes either integer noise, or controlled float (RTE, a fixed denorm mode, no
  contraction, no division or transcendental). That is the GPU rule the worlds research
  proposed for D-016, not yet taken.

**Proposed, in three steps, each its own go:**

1. **The normal maps on the GPU.**
   - **No determinism question:** the map is shading only, and nothing else reads it.
   - **The pass:** a compute pass on the async compute queue (D-020, #77) writes a tile's 256²
     map and its mips straight into the tile's texture. There is then no upload, and no map in
     the world cache (most of its 1.1 GB today).
   - **Its inputs on the GPU:** the Earth's ETOPO pyramid (467 MB at its finest level, about 620
     with the levels above) and the regions (86 MB for the Alps at 90 m). Either all of it, or only
     the levels and windows near the camera; the owner's call on memory.
   - **Precision:** the directions and the octaves' coordinates in `f64` (`shaderFloat64`, on the
     5070 Ti and RDNA; Forge does not enable it yet), or tile-local `f32` offsets; the rest in
     `f32`. A double at 1/64 rate costs little at 197 000 samples.
   - **The check:** each map against the CPU's map, with ꟻLIP under D-017's class thresholds,
     not to the bit.
   - **Expected:** well under a millisecond of GPU a tile (to measure); 50 ms of CPU less a tile,
     the wait about 45 ms.

2. **The heights on the GPU, only under D-016's GPU rule.**
   - **Why the rule:** the physics' height fields and a server need the same bytes.
   - **The code:** the GPU version in integer noise or controlled float, checked by digests
     against the CPU's `dmath` on the 5070 Ti and the AMD iGPU (`FORGE_GPU=amd`).
   - **The use:** the heights are read back for the DAG and the physics.
   - **What it saves:** about 16 ms of CPU a tile. Without the rule, the CPU keeps the heights.

3. **The DAG, the last 35 ms: measured before it is decided.**
   - **The measure:** a tile of one level, its grid's 7 × 7-quad clusters with no simplification
     inside the tile, the quadtree as its only LOD, on a cut one level finer. Measure the GPU
     frame, the triangles drawn, the rays' cut and the swaps' ꟻLIP against today's.
   - **If it holds:** cooking a tile becomes making its clusters and their bounds, a few
     milliseconds on the CPU, or on the GPU behind step 2. That is the "milliseconds".
   - **If it does not:** the CPU DAG stays. The DAG template set aside earlier (23 ms) could still
     be shown while the tile's own DAG cooks, checked with `--check-swaps` for the second swap.

**What the owner decides:**
- the go for step 1 (and how much elevation data the GPU keeps);
- whether D-016 takes the GPU rule, for step 2;
- the go for step 3's measure.

**What it does not change:** D-056's tiles, cut and swap rule; the meshes' cache; the island; the
CPU path's determinism.

**Step 1 built ✅** (2026-10-10, the owner's "let's start that first"; `docs/demos/planet.md`, "The
normal maps on the GPU"):
- **The passes:** `shaders/planet_normals.slang` makes a new tile's map and its mips in passes of
  the frame (`planet/normal maps`, `planet/normal map mips`), declared before the scene's.
  - The tile worker makes the map's image and queues its job.
  - A frame makes at most 8 maps, except the first, which makes the whole first scene's.
  - A new cut waits until its maps are made.
- **On the GPU for the whole run:** the elevation, 703 MiB (the Earth's pyramid and the Alps' at
  90 m), uploaded in 53 ms. The owner's memory choice was the whole pyramid.
- **Precision:**
  - In doubles: the samples' directions and positions, and the coarse and polar tiles'
    latitude and longitude (musl's `atan`).
  - Everything else in floats, from what the CPU works out once a map at its centre: the
    offsets in metres, the angles, each elevation level's place and each octave's lattice.
  - `shaderFloat64` is a new optional device feature. Without it, the maps stay on the CPU.
  - `GraphImage` now keeps its state in a thread-safe cell, so the worker can make an image the
    frames import.
- **Checked against the CPU's maps** (`planet --check-normal-maps`): from level-0 tiles (a pole
  included) to level-14 tiles by Èze, every channel within one 8-bit level, the coast too, and
  the normals within 0.6–0.8°: the encoding's step.
- **The cost:**
  - **GPU:** 0.21 ms a map; the first scene's 378 maps take 80 ms in its first frame.
  - **CPU:** 50 ms a tile less.
  - **The first start by Èze:** its tiles cook in 8.0 s rather than 10.9.
  - **The cold tour:** a cut waits 0.40 s rather than 0.66–0.77, and the tour makes 80 swaps
    rather than 55–62.
- **Kept:** the CPU's maps, as the reference, the fallback and `--cpu-normal-maps`. Their
  entries in the world cache are no longer made and age out with its sweep.

**Step 2 accepted ✅** (2026-10-10, the owner's go: "we will need step 2 sooner or later"), under
D-016's amendment of the same day:
- **The planet's height function is made again in integers,** one definition for the CPU and the
  GPU (`forge_core::fixed`):
  - **The samples' directions:** the equi-angular warp's tangents become sines and cosines, with
    no division: `N cos a cos b + U sin a cos b + V cos a sin b`. They come from integer CORDIC
    rotations once per tile column and row, and are normalised by Newton's iteration in fixed
    point.
  - **Latitude and longitude:** by CORDIC vectoring (shifts and adds).
  - **The elevation's taps, Catmull-Rom, the noise and the blends:** the octaves' lattice, fade
    and gradients, the detail and the regions' blend, all in fixed point. The hash is already
    integer.
  - **The heights:** in fixed-point metres.
- **The CPU makes the tiles' meshes from it,** as a server or the physics will. The GPU makes a
  tile's heights in a pass and reads them back for the DAG, where the CPU spent about 16 ms of a
  core a tile.
- **Its digest test** compares a tile's heights made on the GPU with the CPU's, bit for bit, on
  both GPUs of the dev machine.
- **The normal maps** take their heights from the same integers.
- **To measure as it is built:**
  - how far the integer heights stand from today's floats (under a millimetre is the aim);
  - the CPU's cost a height against today's 0.25–0.33 µs;
  - the GPU's cost a tile and the read-back's wait.

**Step 2's first two parts built ✅** (2026-10-10):
- **The heights in integers** (`forge_core::fixed`, `Planet::fixed_height`):
  - the CORDIC (56 steps) for the warp's sines and cosines and for the latitude and longitude;
  - Newton's iteration for the directions' length;
  - Q61 for the angles and directions, Q32 metres for the positions and heights;
  - the tiles' constants fixed once a tile from the world's floats (`FixedHeights`).
- **Against today's floats,** 5 600 samples on every face at levels 4 to 14: the directions within
  2.7e-16 and the heights within 0.058 mm. A height takes 571 ns on one core, against 485 in
  floats.
- **The tiles' meshes and the CPU's normal maps are made from them:**
  - a fine tile's heights take 5.2 ms waited, against 3.6;
  - the first start's tiles cook in 8.2 s, against 8.0.
- **On the GPU** (`shaders/fixed.slang`, `shaders/planet_heights.slang`), checked by
  `planet --check-heights`:
  - four tiles of 67 081 samples each, from level 14 by Èze to level 8;
  - every height the same bits as the CPU's, with the same digests, on the RTX 5070 Ti and on
    the AMD iGPU;
  - about 0.07 ms a tile on the 5070 Ti against about 38 ms of a CPU core, and 8.5 ms on the
    iGPU.
- **Integrated GPUs keep the CPU's normal maps** (a new `discrete` device feature). The iGPU's
  first scene of 378 maps outlasted Windows' 2 s timeout.
- **The tiles' meshes from the GPU's heights ✅** (the same day):
  - the tile worker asks the frame for a cooked tile's heights;
  - a frame makes at most 8 tiles' heights (`planet/tile heights`), and they come back when
    its slot comes round again;
  - the worker builds the mesh from them (`tile_mesh_from`), the same bits as its own;
  - before any frame (the first start) or once the frames stop, it makes them on the CPU.
  On the cold tour, a cut waits 0.39 s against 0.48 with the CPU's integer heights (one run each).
  `--cpu-heights` keeps them on the CPU.
- **The normal maps from the heights in integers ✅** (the same day):
  - a map's 513² samples come from the heights pass (16 maps at a time), their directions are
    made again in integers, their points in doubles;
  - the float version of step 1 is gone (its centre-relative angles and noise, its double `atan`);
  - against the CPU's maps, from levels 7 to 14: within one 8-bit level, as before;
  - 0.25 ms of the GPU a map, against 0.21 in floats.

**Step 3 measured: the DAG stays ✅** (2026-10-10, the owner's "go with step 3";
`docs/demos/planet.md`, "One level a tile, measured"; the code in
`reports/2026-10-10-220/one-level-tiles.patch`):
- **What was built for the measure** (`planet --one-level-tiles`):
  - each tile's grid cells as its only level, every cluster a root, nothing simplified;
  - for the rays, the tile's grid at 45 × 45 samples (3 872 triangles, within the 4 000 a tile's
    DAG cut takes), handed to the scene rather than cut from a DAG;
  - never cached.
- **A tile cooks in about 9 ms waited** rather than 37 (its heights apart), 1 403 clusters in
  14 pages rather than about 3 170 in 30. The first scene's 369 tiles cook in 2.0 s rather
  than 8.3.
- **But the frames draw 12 to 100 times the triangles.** The GPU frame, best of three runs:

  | View | The DAG (today) | One level, the same cut | One level, 129 samples a tile, the cut one level finer |
  |---|---|---|---|
  | Èze from the ground | 0.94 ms, 133 k triangles | 1.19 ms, 3.8 M | 1.15 ms, 1.6 M |
  | 10 km up | 1.12 ms, 331 k | 1.96 ms, 21 M | 1.76 ms, 12 M |
  | Orbit | 1.21 ms, 383 k | 2.53 ms, 40 M | 2.15 ms, 24 M |
  | Mont Blanc from 6 km | 1.10 ms, 332 k | 1.54 ms, 8.9 M | 1.63 ms, 6.0 M |

  The software rasteriser takes most of it: 0.55 ms at 10 km and 1.06 ms from orbit with one
  level, where the DAG's frame has none. The quadtree places a tile's samples about 2.5 px
  apart at its nearest point. The DAG then removes most of them where the ground is gentle and
  the tile is far, which the quadtree cannot do inside a tile.
- **The rays' grid errs 2–3.5 times the DAG's cut** (5.9 m against 2.9 at level 14), so their
  rays would start further off the ground. The smaller tiles double the rays' structures (193
  against 83 MiB) and the tiles (822 against 369).
- **So the CPU's DAG stays,** 35 ms of a new tile's 45 waited. The swaps' check on the tour was
  not run: the frames decide it already.
- **Then the tile's coarser levels made from its grid,** measured and set aside the same day
  (the owner's "try the coarser levels made from the grid", then "set it aside as a patch";
  `reports/2026-10-10-220/grid-dag.patch`):
  - **How:** level `k` keeps every `2^k`-th sample inside groups of 16 × 16 cells, each group's
    border as its children drew it (the border's cells fanned), the groups shifted half a group
    each level so that the borders coarsen at the next; level 0 in blocks of 8 × 4 quads; the
    errors exact against the tile's samples. A unit test found every cut watertight.
  - **It cooked a tile's DAG in 16 ms rather than 36–38,** in 5 levels and 23–24 pages.
  - **But the frames drew 1.6–5 times the triangles,** the grid's levels erring 3–4 times
    meshoptimizer's at as many triangles: the GPU frame 1.00, 1.32, 1.43 and 1.27 ms (Èze, 10
    km, orbit, Mont Blanc) against 0.94, 1.12, 1.21 and 1.10, the clusters' culls most of it.
    The rays started further off the ground, their errors larger, and some near shadows shrank.
  - **Set aside:** the frames pay all the time, for 20 ms of a cut that waits about 0.4 s on a
    cold tour. Levels that halve (one axis at a time) would draw fewer triangles, not likely as
    few as meshoptimizer's.
- **Left, if a cut's wait matters again:** the DAG template set aside earlier (23 ms), shown
  while the tile's own DAG cooks and checked with `--check-swaps` at the second swap. Not
  proposed yet.

## D-059 — A city sandbox drives the engine: a private game-quality city on public engine work ✅ (accepted on the recommendations, 2026-10-11; confirmed by the owner the same day, to be improved as we go)

**The question:** the owner's direction of 2026-10-10. After the planet work (D-058), the next
showcase driver is a city sandbox built as if for a game release, with Vuntra City as
inspiration rather than method:
- **The place:** a city by the sea with a river, parks and landmarks. Every building, home and
  structure is enterable, with an interior that fits its use.
- **The people:** a persistent population of at least a million, each with a home, work,
  appointments and a personality. Multiplayer from the start, with the owner's second PC (an
  i7 and an RTX 3080) as the server.
- **The player and transport:** first person, and public transport.
- **The look:** release quality and one consistent style.

The owner refined it the same day:
- **The look:** not futuristic. A current European city a few years ahead, modernised and mixed
  much more with nature, with a few landmarks.
- **Transport:** all electric public transport. Trams; *public* automated taxis that may share
  the tram tracks where they are laid flush (Nantes); river taxi-boats crossing at several
  points (Navibus). No private cars, less room for roads, more for people.
- **Technology:** integrated with the city and nature, people's well-being first, no fossil
  fuels.
- **Thirteen ideas accepted:**
  - the real site chosen now;
  - the smart pad as the player's interface;
  - live data on the city's screens;
  - darker nights with stars;
  - a quiet city;
  - weekly events;
  - regulars and a friend group;
  - streets as play spaces;
  - visible nature;
  - rooftops;
  - swimming;
  - a "day in the city" flight as the regression check;
  - a style bible as data.

The owner's instruction of 2026-10-11 was "Follow recommendations for now. Tomorrow sum up the
choices you made. Start implementing when you are ready." So this entry records the research's
recommendations as accepted, and each one stays open to the owner's review. The choices made
on the way are listed for that review in the sandbox (`docs/choices-2026-10-11.md`).

**The research** (2026-10-10, one agent at a time):
- `assets-and-licences.md`;
- `population-simulation.md`;
- `procedural-content.md`;
- `city-generation.md` §7–§19;
- `people.md`;
- `transit-and-vehicles.md`.

Each note's city-specific appendix stays in the sandbox.

**Accepted:**

1. **The publication split** ✅ (the owner's rule, binding from the first file):
   - **The engine stays public.** Generic engine work goes into `forge-engine` as usual, through
     its issues and decisions: layout, grammar, interiors, crowds, transit, simulation tiers,
     netcode.
   - **The sandbox stays private.** Its code, data, generated content, city research and any
     bought or licence-restricted assets live in `../city-sandbox/`, a local git repository with
     no remote (the owner's choice, its C-001). It uses the engine's crates by path and is never
     pushed; a private remote comes only on the owner's go.
   - **Plans:** city-specific plans live in its local `docs/TRACKER.md`, not on the public
     tracker. The engine's own demos use placeholders.
2. **Scope and scale:**
   - The first city is about 8 km across for a million people, on a flat sandbox.
   - Its coast and river come from the real ground: the Argens plain at Fréjus, from Forge's
     GLO-90 mosaic. Of the three sites measured, it is the only one with room for 8 km of flat
     land. So moving the city onto the planet later is a placement, not a redesign.
   - Four million people means a larger map (about 16 km), not a denser city.
3. **Art direction and the first style set:**
   - **The look:** the owner's direction above.
   - **The first style set** ✅ (the owner, 2026-10-11): a Mediterranean-European perimeter
     block. This amends D-039's order: the city comes before the medieval village.
   - **Consistency:** a style bible as one data file (palette, heights by street width,
     cornice lines, opening rhythm, green scores, the 3-30-300 rule, lighting), which every
     generator reads and checks its output against (`city-generation.md` §15).
4. **Transport:**
   - **Trams.**
   - **Public automated taxis** on paved tram lanes and service streets. In the car-free core
     they drive only at walking pace, for reduced mobility and late night; never on grass track.
   - **River taxi-boats** and a river line.
   - **Bikes everywhere.**
   - **Service vehicles** filtered by class and hour (Ghent, Pontevedra): hubs and cargo bikes,
     morning windows, quiet night trucks.
   - **Emergency access:** a 4 m strip along every facade.
   - **No private cars.**

   The transit simulation follows `transit-and-vehicles.md`:
   - **Trams and boats** are timetabled runs whose position is a pure function of their schedule,
     delay and speed profile. People board and alight as events. Lines run by holding to their
     headway, with a dwell cap.
   - **Taxis** are a dispatcher from fixed stands, with insertion, rebalancing and a quoted wait.
     They enter tram track only in a booked gap.
   - **Near players:** trams kinematic on bogie points, taxis with the Intelligent Driver Model,
     boats as Jolt bodies.
   - **The pad** routes with RAPTOR.

   With the clock below, the taxi fleet is capped at 4 000–5 000 for about 2 % of trips
   (accessibility, night, luggage), and the boat crossings run every 2–3 real minutes.
5. **Asset policy** (`assets-and-licences.md`):
   - **Four licence classes.** The public repository holds only class 1, through D-048's fetch
     (never committed).
   - **Class 2** (MetaHuman, Fab and Megascans, KitBash3D, Substance assets, trees from a tree
     tool) lives in the sandbox's ignored library, with a manifest of licence, conditions and
     date.
   - **A release** packs its assets protected, under its own data licence.
   - **Learned models** train on class 1 or Forge's own data only.
   - **Quaternius** counts as class 2 until a fresh download's licence file says CC0.
   - **Nothing is bought or downloaded without the owner's go.** The paid shortlist waits for
     the owner's review.
6. **Procedural first** (`procedural-content.md`):
   - full PBR materials with wear derived from the geometry, plus trims and decals;
   - objects as part programs;
   - a cook stage (clean, unwrap, bake) into the D-053 cache.

   Material Maker (MIT) and headless Blender Geometry Nodes run offline. Generative 3D AI
   stays out of the engine path (D-016; Hunyuan3D's licence excludes the EU).
7. **The population and the server** (`population-simulation.md`):
   - **The tiers:**
     - everyone as a compact record advanced by events (T3);
     - coarse agents near players (T2);
     - full agents within about 150 m (T1);
     - the drawn crowd is the real population (T0).
   - **The server's CPU decides everything** (D-016). The RTX 3080 works offline: generation,
     calibration, fast-forward tests.
   - **Persistence:** snapshots of the changed columns plus an input log.
   - **Interest:** by cells and a priority accumulator (D-010), not by entity-held query
     components (Improbable's patents).
8. **People** (`people.md`):
   - **The sandbox** uses MetaHumans (class 2), read by a Rust port of the MIT DNA and RigLogic
     code.
   - **The public demos** use class 1 placeholders (MPFB, Anny, Rocketbox).
   - **LOD:** skeletal near, baked bone-animation crowds, impostors to 1.5 km.
   - **Clothes:** fitted by offline binding and morphs.
   - **Dialogue:** carried as line ids.
   - **No memories that spread between people** without the owner's go (Warner Bros.' Nemesis
     patents).
9. **Time:** one game hour is 20 real minutes (a day lasts 8 hours; `population::GAME_SPEED`).
   It was 15 minutes until the owner's second walk (2026-10-11). People near the player walk
   at their real pace, which the owner found right. The transit research's finding:
   - **Capacity shrinks.** With vehicles at real speeds, a peak game hour lasts 20 real minutes,
     so any line or fleet carries about a third of what it would per game hour in life. Capacity
     is planned in real time against demand in game time.
   - **The expected shares:** walking about 38 %, cycling 34 %, trams 22 %, taxis 2 %, boats
     1.5 %.
   - **Commutes:** a median commute of 45–60 game minutes door to door.

   For the owner's review: keep the clock and these shares, or slow the clock, or add a light
   metro trunk.
10. **Testing, for a while** (the owner, 2026-10-10):
    - **The regression check:** the sandbox's checks and its "day in the city" flight replace
      the Tier 0/1/2 GPU batch. Running the simulation and every tier for each change would
      take too long.
    - **The gate stays** on every push: build, tests, clippy, formatting, credits, as CI.
    - **Labs** stay for experiments before they go into the engine or the sandbox.
    - **The demos** still build and test. Their captures run when a change touches what they
      draw.

    `tools/impact.toml` maps the new city paths to the gate.
11. **Patents and giving back** (the owner's rules, `CLAUDE.md` and `CREDITS.md`):
    - every note flags the patents it meets;
    - decisions prefer unpatented or expired methods;
    - a live patent waits for the owner's go;
    - part of any profits goes to the free projects a release leaned on.
12. **The steps,** each ending in a visible, measured, verified result:
    1. **The city's plan on the flat sandbox, judged as a map.** Built 2026-10-11:
       `forge-procgen::city`, `docs/demos/city-plan.md`. The sandbox's first map is of the
       Argens.
    2. **Buildings** from the grammar and kits in the first style set, far proxies, materials
       of release quality. *Begun 2026-10-11:*
       - each lot's mass (the grammar's first stage), its floors capped by the style bible
         (#246, `city::bible`);
       - facades as textures of window bays and shopfronts, drawn in 3D by `demos/city-plan`;
       - D-039's first build (#247): the component split (`city::facades`), the split grammar
         with its text form (`forge_procgen::grammar`), the first style set's rules and kit
         (`forge_geom::modules`), the buildings near the view drawn from them.

       Since then:
       - tiled roofs in the old towns, each row under one ridge with gables on the party
         walls;
       - no sliver masses;
       - squares and five landmark models (market hall, terraced hall, gantry crane,
         pavilion, weir bridge).

       Next for #247: the module grid, a proxy per building.
    3. **Vegetation:** trees, grass, parks. *Begun 2026-10-11:* `city::trees` places rows
       along the streets, trees in parks, on quays and in courtyards, drawn as one instanced
       low-poly tree.
    4. **The player** walking the streets in first person; interiors furnished by use, in tiers
       (`city-generation.md` §17). *Begun 2026-10-11:* walking in first person in
       `demos/city-plan` (Enter), a Jolt character among the masses as walls.
    5. **Night:** many local lights. *Previewed 2026-10-11:* lit windows from emissive maps.
       The probes flood the scene with that light (#254).
    6. **People:** the runtime, crowds at scale, navigation, then lives and schedules. *Begun
       2026-10-11:* the population on foot round the camera as placeholder figures, on routes
       along the streets (`demos/city-plan --people`), until #251's humans.
    7. **Public transport:** trams, taxis, boats; then vehicles to pilot. *Begun 2026-10-11:*
       trams as timetabled runs (`forge_sim::transit`), a part-program model
       (`forge_geom::vehicles`), drawn as movers on double tracks in `demos/city-plan` (#253);
       a river shuttle boat at each crossing, on a two-stop timetable; people boarding from
       the platforms; lines ending at one hub staggered.
    8. **The population server,** scaled to 1 M and beyond; multiplayer clients. *Begun
       2026-10-11:* `forge_sim::population` (#250): the day of 4 M people on six workers with
       one digest, and snapshots of the running state.
    9. **Ambient audio.** Later: ships, aircraft, drones, weather, third person, LLM dialogue,
       visemes.

*Not chosen:*
- **The Var's mouth as the site:** about 20 km² of flat land, so the city would climb the hills
  in terraces.
- **A sandbox folder inside this repository:** one wrong `git add` would publish it.
- **Generated floor plans from learned models:** research-only licences.
- **GPU-side population ticks on the server:** D-016, and servers rarely have a GPU.

*(research: assets-and-licences.md, population-simulation.md, procedural-content.md,
city-generation.md §7–§19, people.md, transit-and-vehicles.md; issues #83–#90, #168, #222,
#224, #231, #234, #39)*

## D-060 — Buildings carry their structure: storeys, walls with their thickness, openings, cores and units ✅ (accepted on the recommendations by the owner, 2026-10-11; to be worked on first, as it shapes the rest)

**The question:** the owner's notes during D-059's sandbox work (2026-10-11). Walls have a
thickness, and old stone houses thicker ones than recent buildings; trams too. The floors
between storeys have a thickness. Heights, widths and lengths must add up for the structures
to stay consistent. A varied kit of modules may simplify physics and destruction later, as in
Battlefield 6. And the interiors, by use, are wanted to judge the buildings' sizes (#248).
Today a building is a mass (a footprint, a 4.2 m ground floor and 3.2 m upper floors), dressed
with a facade kit; nothing has a thickness, and the population counts gross area at 40 m² a
person.

**The proposal** (`research/building-structure.md`):
1. **A building's data holds its structure:**
   - its storeys: level, floor to floor, slab thickness, clear height;
   - a wall graph: segments with their thickness, material and role (façade, party, courtyard,
     core, partition), and openings with sill, head and reveal;
   - a core: stairs and lift;
   - its units: use, type, rooms.

   The façade grammar puts its windows into those openings, their reveals as deep as the wall.
2. **A 0.30 m planning module** (a 0.90 m door is three, a 1.20 m corridor four), the façade's
   outer face on the building line, the thickness inwards. Thicknesses and heights by style:
   - old stone town: façades 0.60 m, party walls 0.50;
   - Haussmann and the 1800s: 0.45 to 0.40 and 0.40;
   - modern: 0.35 and 0.20;
   - slabs 0.27–0.30 m;
   - the floor-to-floor heights in the style bible per style, the note's table.
3. **Checks as tests:** the storeys add up to the height; gross area (outer faces) and net area
   (the rooms) are both kept, net over gross about 0.80 modern and 0.70–0.75 in old stone.
4. **The population from net area** (or by dwelling type): 32.4 m² a person, INSEE's figure, in
   place of 40 m² gross, which is right for modern buildings and too many in old stone ones.
5. **Accessibility as generator rules:** lifts from R+3 and in shops and offices with public upper
   floors, type 2 cars (1.10 × 1.40 m), 0.90 m doors, 1.20 m corridors, step-free entrances (#263).
6. **The order:**
   1. the data, the tables and the tests, the population retuned;
   2. the façade kit reads the walls, and collision is made from walls and slabs;
   3. cores and units on the grid, interior mapping fed by the same rooms, then enterable
      interiors by use (#248);
   4. destruction: a panel graph (a bay of one storey), connectivity to the foundations,
      pre-cut chunks as Jolt debris, the state on the server.
7. **The trams** get the same treatment in their model: walls of 60–100 mm, a floor 350 mm over
   the rails, about 2.2 m clear inside (done roughly on 2026-10-11).

**Patents to clear first, flagged by the note:**
- **Sony US 8,152,613** (to about 2030): stress nodes per tile breaking over a threshold. An
  overload pass waits for a patent check and the owner's go; connectivity to anchors differs.
- **Celsys US 12,700,170** (to about 2042): full rendering of the room one is in, interior mapping
  for the others, swapping as one moves, a see-through door. Avoid that combination, or ask.
  - **Claims read 2026-10-14** (granted 2026-08-04): claim 1 is the only independent claim. It
    needs all three elements: interior mapping for the spaces one is not in; a passable door drawn
    as if partly transparent onto them "to present that the door is passable"; and the swap as
    one crosses. Windows are not in the claim. The owner's plan has no see-through door (interior
    mapping behind windows, real rooms near, opaque doors, F builds the interior). Rule: never
    draw a door see-through onto interior-mapped rooms. This is our reading, not legal advice.

**Panels or data:** the note's lean, taken with the recommendations: panels built from the wall
graph (one body set per bay and storey), so destruction and collision share one structure, the
kit dressing them.

**The owner's go (2026-10-11),** with notes for the structure system:
- openings come from the walls: no window where another building covers the wall (windows cut
  by a neighbour were "not realistic at all"), and storeys that add up (a squashed row of
  first-floor windows);
- every block's inner court reached through a door or a passage from the street;
- lit windows from the people living there, by what they are doing (the population's
  activity, not a texture);
- smaller courtyards as communal gardens the neighbours tend, and real parks here and there;
- the patent checks before the steps that need them: Sony's tile stress before any overload
  pass (step 4), Celsys's room swapping before the interiors' rendering (step 3).

**Ground floors (the owner, 2026-10-15).**
- **The owner's note:** some ground-floor flats have a garden (*rez-de-jardin*), others are
  plain (*rez-de-chaussée*). Round a green court they usually have a glazed bay, and a shared
  kitchen garden is likely. Not every ground floor on a pavement is a shop: shops go on the main
  roads, near the sights, in the centre, along the coast and the rivers.
- **Research:** `research/ground-floors.md`.
- *Built 2026-10-15:* which ground floors are shops comes from a chance by street and frontage,
  with modifiers for the centre, the tram stops and the sights, drawn by stretches of street so
  that shops form parades. The example has 31 shopfronts per 1 000 people (Paris 28).
- *Built 2026-10-15 (`city::gardens`):*
  - Paths cross each green court from its gaps, passages and court doors to its middle.
  - Its ground-floor flats line its walls, 8 m each between the doors. Half have a hedged
    garden with a gate, the others a terrace. Each opens onto its terrace through a glazed bay
    to the floor.
  - Planters stand before the homes on the streets, broken at the doors.
  - Each court of 600 m² or more has a kitchen garden of raised beds 0.7 m tall on paths of
    1.4 m.
  - All of it is step-free. The example has 9 106 garden flats, half with gardens, and 277
    kitchen gardens.

**Step 3's rooms behind the windows (built 2026-10-15).** The kit's glass shows a room traced in
its shader: interior mapping, with walls, a floor, a ceiling, a picture, a door and a piece of
furniture, each window's own draw, and net curtains on some. A lit row's room took 4 % of its
luminance (a quarter since, below), so the room shows at night instead of a white pane. Windows
only, so Celsys's claim is not met: no door is ever drawn see-through. The rooms are not yet the
units' rooms: the same box for every window (`demos/city-plan.md`, "Rooms behind the windows").

**Rooms as pictures of rooms (built 2026-11-06, `RoomAtlas`).** A window can show an authored kit's
baked pictures in place of the drawn room: `RenderLayer::room_atlas` gives the atlas, its rooms
across and down, and a room's size in the mesh's metres.
- **The ray:** it meets the room's box behind the glass (`interior_atlas`). The point it meets goes
  to the picture as the picture was taken from the glass, narrowing towards the back wall.
- **Which picture:** a room's own, by a hash of where it stands.
- **Its light:** the row's `emissive` lights it whole, and no curtain is drawn. A far window reads
  the mip level its pixel covers, down to the atlas's mean.
- **Rays:** a mirror or a bounce that meets it takes the pictures' mean under the same lamp, so
  they agree with the view.
- **The row:** it needs `interior`, and it reads no UVs, since the rooms stand in the mesh's metres.
- **Other rows:** a row without an atlas draws its room as before.
- **Patents, read 2026-11-06 (our reading, not legal advice):**
  - **Celsys US 12,700,170:** windows only, so its claim is not met, as above.
  - **Bosch US 10,380,790:** claim 1 starts from a façade texture's window cutout layer and the
    floors found in it. Here the grid stands in the mesh's own metres at the row's given size, and
    no façade texture is read. Only the opening of claim 1 could be read (the page cut it off).
  - **Sony US 12,293,452 (to about 2039):** its claims need planar layers parallel to the window,
    duplicated and resized for a second room, and two windows on one connected room with a
    parallax offset between their layers and reflections shaded from it. Here each room is one
    picture, with no layers and nothing shared between windows. Reread those claims before adding
    a layer atlas (curtains drawn as planes behind the glass).

**Lit by their residents (built 2026-10-15).** With an hour, each home's windows light by its
household: at home and awake or not. The day plans say where people are, and a sleep model was
added: waking before the first outing, bed at the role's hour or after coming home. A window
finds its flat among its storey's units, and the flats take the building's households in turn.
At 21:00, 88 % of the kit's households are up, and 49 % of its windows are lit; at midnight,
15 % and 10 %. The share of the hour remains for the shops and as the fallback. A lit room takes
a quarter of its lamp, not 4 %: under the probes' street lamps no window looked lit from the
pavement (`demos/city-plan.md`, "Lit by their residents").

**Offices and homes upstairs (built 2026-10-15).** Step 3's first item: uses go by floor and
frontage (`research/city-generation.md` §14).
- **Where:** offices over the shops on the main streets (the first floor, the second over five
  storeys), and whole office buildings by the tram interchanges, by a chance per lot drawn by
  block. About 14 % of the upper floors in the example, 42 % within 400 m of an interchange.
- **The population counts by home:** each occupied flat is a household (5 % stand empty). Its
  people are its rooms at INSEE's 1.5 rooms a person in flats, about 1.9 a flat. The jobs are the
  offices' floor at 15 m² an employee and the shops' at 25 m².
- **The result:** the sandbox keeps its million (1 043 k in 585 900 homes, 576 k jobs). Office
  windows light by the working day, and the residents' windows by their own household.

`demos/city-plan.md`, "Offices and homes upstairs".

*(research: building-structure.md; ground-floors.md; issues #248, #263)*

## D-061 — The street ground as one map: cross-sections, junctions first, every surface once ✅ (proposed 2026-10-14; the owner's go the same day, in the order given)

**The question.** In the owner's walk (2026-10-11) most of the bugs came from the street ground:
- crossroads wrong wherever a pedestrian way meets the streets;
- stops in junctions or on top of each other;
- paths under buildings, or leading nowhere useful;
- paved areas nobody can explain;
- geometry over the water;
- isolated islands of city while other areas stay empty.

The owner (2026-10-14): "we will have to re-work the road/sidewalk system as there are a lot of
bugs related to it".

**What we have** (a survey of the code, 2026-10-14):
- **The road graph** stores a centreline and a `Profile` per edge, with only a width and a pavement
  width.
- **The demo** draws each surface as its own patch: a ribbon per edge, pavement strips, ramps,
  paved corners in several cases (a swap, a straight band, bend wedges, a cut for the trams' swept
  path), promenades, squares and court paths.
- **Overlaps** are kept apart by 1 cm height steps: 0.16, 0.17, 0.18, 0.30 and a dozen more.
- **"Raised or not"** is decided four different ways.
- **Blocks** are faces of the centrelines, moved in by a width read from the nearest street sample.
- **Walkability** is computed three ways, and none of them reads what is drawn:
  - the crowd walks the graph;
  - the reach check uses its own raster;
  - the first-person walker stands on a flat slab, so feet sink 14 cm on pavements.
- **Stops** are snapped to junctions by the plan, then moved off them by the demo: two rules
  that fight.
- **Bridges** are found from an edge's midpoint.
- **Districts** take every land cell in the frame, with no distance limit: the hills alone decide
  where the city stops.

Each bug fixed so far has been a local patch on this, and the next junction type breaks again.

**The proposal.** It follows how city tools generally build streets (CityEngine's street shapes,
for one): junction areas first, then the street between them, then the blocks from what is left.
Every surface is drawn once.

1. **Cross-sections as data.** A profile is a list of strips across the street, each with:
   - its width and surface;
   - its level: carriageway, kerb top, or flush;
   - who uses it: walkers, bikes, taxis and service vehicles, trams;
   - its furniture, trees or none.

   The profiles follow `research/city-generation.md` §12, including the tram avenue at 26–32 m with
   cycle tracks. Trams get a little wider (about 2.65 m, the owner asked), and the swept path of
   two trams passing on a curve is checked.
2. **Junctions first.** At each node, the arms' cross-sections are met:
   - the carriageway core with kerbs turning on their radius;
   - the pavement corners between the arms;
   - a crossing on each driven arm, with dropped kerbs or a raised table.

   Each kind of meeting is one rule, not a patch:

   | The arms | The junction |
   |---|---|
   | driven × driven | a core with crossings |
   | driven × pedestrian way | a continuous footway: the pavement level runs across the street's mouth |
   | pedestrian × pedestrian | one flush plaza |
   | a bend (two arms) | the strips mitred, their offsets kept true on the curve |

3. **One ground map.** The streets between the junctions are each strip trimmed at the junction's
   edge.
   - Blocks are what is left: the building line is the pavement's real back edge.
   - The map partitions the ground. Every point is exactly one region with one level: carriageway,
     pavement, crossing, platform, path, plaza, court, lot, green, quay or deck.
   - Kerbs and ramps are made where two regions' levels differ.
   - The height steps go, and aprons and seams with them.
   - Construction is direct, with no general polygon booleans. If they turn out to be needed,
     `i_overlay` (MIT) is the candidate, after asking the owner.
4. **Everyone reads the map.**
   - **The pedestrian network:** footways, crossings, paths, court paths, passages and plazas, made
     from the map.
   - **Its users:** the crowd, the first-person walker's ground heights, and the reach check.
   - **Rules:** paths keep clear of the masses, by a metre or two and more by ground-floor windows,
     except through real passages.
5. **Paths from people's trips.** After the buildings and stops, doors are routed to their stops,
   shops and parks over the walkable ground:
   - paths are laid where the routes gather (the desire lines planners follow);
   - paths nobody uses are dropped;
   - a check measures how direct each door's walk to its stop is.
6. **Stops in the cross-section.** A stop is a stretch of the street whose section carries a
   platform. The plan places it once, under one rule:
   - shared by every line on that stretch;
   - never within a junction's reach, on a curve or on a bridge;
   - the tram's swept body tested against the platform's furniture.
7. **Bridges and water.**
   - A bridge spans the edge's actual wet stretch.
   - Two bridges close side by side become one.
   - A check finds any ground-map region over water that is not a deck or a quay.
8. **Where the city is.**
   - Districts get an outer limit, with countryside beyond it.
   - A built area must connect to the centre by road. If it doesn't, it gets a reason to be there
     (a village, a port, a station), or it isn't built.
   - Density falls with the distance from the centre and the tram stops.
9. **Checks as tests:**
   - the ground map's regions don't overlap and leave no gap;
   - every footway is connected;
   - every driven arm of a junction has its crossing;
   - no path runs under a mass or within its clearance;
   - the stop rules hold;
   - the walk to a stop is direct;
   - nothing stands over water but decks and quays;
   - no isolated fragment remains.

**What stays:** the street tracing, the graph's cut and pruning, the faces and blocks (now inset
from the ground map), the masses, the structures (D-060), the kit and the trams' timetables.

**Order:** 1–3 first, with the demo drawing the map, the old patches deleted as their cases move
in, and the captures compared. Then 4 and 6, then 5, 7 and 8.

**The street design, the owner's answer (2026-10-14),** after seeing the first map. "junctions,
road intersections and bridges are still a mess": the map drew the network faithfully, and the
network itself is the mess (crossing bridges, junction clusters, sharp meetings).
- **Roads stay, for the taxis; the taxis stay off where people walk.**
  - The main streets are normal two-way roads, a little futuristic but not too much, narrower:
    automated taxis hold a 3 m lane.
  - No one-way streets: a taxi must not tour the city to reach the next block. The main roads'
    spacing keeps detours short.
  - Tram avenues carry the tracks and a taxi lane each way.
- **Every other street is for people only:**
  - pedestrian streets with bollards at their mouths, deliveries and emergencies at set hours;
  - a continuous footway across their mouths where they meet a road.
- **Intersections only where roads meet:** zebras on raised tables, rounded kerbs, no traffic
  lights.
- **Taxi stations:**
  - stands of 2–4 bays along the roads, by the tram stops, squares, the hospital and the station;
  - depots for charging and cleaning in the industrial quarter;
  - every building within a short walk of a stand, as a check.
- **The vehicles** (proposed): a 4-seat cabin with facing seats as the standard (most trips carry
  one or two people, but a family, luggage or a wheelchair with a companion fit), and an 8-seat
  shuttle with a ramp for groups and accessibility.
- **The network cleaned before it is drawn**, with a defect count:
  - junctions closer than about 20 m merged;
  - near-duplicate streets dropped;
  - sharp meetings straightened;
  - one bridge per crossing, roughly square to the river, no junction on it, a small square at
    each head.

**The taxis, the owner (2026-10-14):** fully automated, booked like Uber or inDrive from a
person's smart pad or communicator; a taxi comes to them. Idle taxis park at set places. At busy
times they circulate round the busy places, where they will most likely find someone to carry.
- So the stands are parking and pick-up places: lay-bys along the roads, by the tram stops,
  squares, the hospital and the station; depots in the industrial quarter.
- A pick-up point within a short walk of every building, as a check.
- *Built 2026-10-14 (`forge_sim::taxi`):*
  - **Dispatch:** bookings go to the nearest free taxi by road. It fetches the passenger,
    carries them, and returns to a stand with a free bay.
  - **Busy hours** (the hour's rate above 1.3 times the day's mean): each stand's second taxi
    cruises.
  - **Trams first** (the owner: not everybody takes a taxi, and it has to be useful to the one
    who does): a trip goes by tram when one line serves both ends within 400 m, or two lines
    with one change. A share of the other trips books a taxi, so that taxis carry D-059's 2 %.
  - **Fleet size:** three trips a taxi in the busiest hour. The example has 515 taxis, with no
    bookings waiting and a 43 s mean wait at 9:00; the sandbox 1 727.
  - **Traffic:** car-following in each lane (the Intelligent Driver Model), one taxi at a time
    through a junction with room past it, stop lines 10 m before. The drawn pose rounds each
    corner between two legs on a 7 m radius through the node (2026-10-16; the owner: the taxis
    turned on the spot), the simulation keeping to the lines.
  - **Depots** (built 2026-10-15): split-level parking decks (177 cabs) behind the building
    lines of the taxi roads, one for 25 stands, an industrial district preferred (counted 800 m
    nearer) but never far from the stands they serve.
    - **Circulation** (the owner asked how car parks keep their flows apart): one way round, an
      up lane and a down lane on each side, the ways in and out apart, columns only between
      bays.
    - **Staff:** a stair and lift core with a door onto the pavement.
    - **Driveways:** they climb the verge, so the footways stay level.
    - **Use:** the cabs no stand holds park and charge there; the depots refill the stands and
      send their cabs out at busy hours.
  - **Shuttles** (built 2026-10-15): 8 seats with a ramp, 5 % of the fleet, for parties of five
    or more.
  - **Not yet:** batteries and charging times; shared rides.
- **Crossings (the owner's go):** raised tables on the taxi roads; dropped kerbs on the tram
  avenues.

## D-062 — Mission maps drive the core: small generated places first, the city as the integration target ✅ (proposed 2026-10-15 on the owner's question; the owner's go the same night)

**The question:** the owner, on 2026-10-15, after their second flight over the sandbox (twenty
sightings with their views, `city-sandbox/docs/TRACKER.md`): "I wonder if we should not change
strategy and reduce the scope and work on different technologies independently in sub sessions
when possible like network, sound system, textures system, and so on [systems for flora and
fauna], like the core thing that we can merge as we progress with the simulations or demos."
The proposal: a first-person game whose maps, small at first, are fully generated with a
mission each (zombies; a village with a military base to infiltrate, soldiers to avoid or to
fight; a Battlefield-like solo fight), detailed enough to go into the places, so that issues
show sooner, the core technologies backported to the city and the demos. "The demos were not
bad but were maybe too small in scope and were getting very long to test when running the full
tests suite with tier 0, 1 and 2."

The owner's goals, the same night:
- in the end, to generate different environments, biomes, planets, solar systems, places,
  weathers, fluids, structures, styles of civilisation (primitive, medieval, futuristic,
  dystopic, cyberpunk-like), with beautiful and physically accurate graphics, loading times and
  performance tracked;
- on rendering: "if required, would go full DLSS 4.5 (or 4 if not available) with DLSS and ray
  reconstruction, ray traced shadows, ray traced reflection, hybrid ray tracing, path tracing,
  hardware flip metering, neural radiance cache, the best we can do, and stop caring for less
  nice rendering methods except maybe the AMD equivalent" (no AMD hardware to test on);
- the city renders many useless things inside its empty buildings (shadows, anomalies).

**The recommendation** (the session's answer): agree, with two changes.
- **Why:** about half of the night's sightings are engine faults (triangles past roofs, smeared
  textures, shimmering faces, grilles, shadow pops, z-fighting, a reflective underside) that any
  detailed place shows, and the city shows slowest: half a minute to load, a million people in
  the way, layout faults hiding the engine's. A small map loads in seconds, is shot from a few
  fixed views in a minute, and can be entered. The tiers grew long because each demo tests one
  feature alone, while the owner looks at everything at once.
- **Change 1, the first milestone is a place, not a game:** one generated village or compound,
  enterable buildings from a structure generator, props, plants, the day cycle, one objective
  (reach a room, find an item), the player walking. No enemies or weapons until the map looks
  right still and moving: combat brings four systems at once (characters' animation, AI,
  weapons, a HUD), each weeks, and they would look bad on a world that still shimmers. When the
  world holds, zombies first (melee, no aim, no cover tactics).
- **Change 2, parallel sessions only off the GPU:** sound, networking and the simulation side of
  flora and fauna (growth rules, animals' behaviour) as CPU-only crates with headless tests, no
  GPU lock, no window on the second screen. Anything that renders (materials, the plants'
  rendering, the map) in one session at a time. Each session owns its crates; the CI queue and
  the one-research-agent rule stay shared; three sessions at most.
- **The check per commit becomes a map check:** build, tests, clippy, then one seed, six fixed
  views and one walked path compared with accepted shots, about two minutes, as the sandbox's
  day check does. Tier 1 and 2 become milestone checks the owner starts (like
  `tools/amd-check.sh`), not the gate (D-043 amended).
- **The city stays the integration target:** its layout faults are hours each (the tram stops,
  the double turn, the too-close walls) for short sessions; its day check runs at milestones;
  the structures, materials and plants built for the maps are backported.
- **The order of the core:** geometry and LOD correctness first (the night's class of faults);
  materials (trim sheets, decals, layering); structures with interiors; flora placement and
  rendering with wind; characters' animation (which also fixes the popping people); then
  sound, fauna and networking. The owner added the particle system the same night (smoke,
  dust, fire, rain, sparks: GPU work, so one session at a time), after the flora.

**The owner's go, the same night** ("We will follow your plan"), with two answers:
1. The licensed things that cannot be redistributed stay in a private repository; the core
   engine stays free of them. So the generators and systems go into forge-engine, public, and
   the scenarios, missions and licensed assets into a private repository, as the city sandbox.
2. The style: the owner thought of a more tropical architecture (the Yucatán, South America,
   Africa, Asia) but starts with what we have: the first map is a Mediterranean village built
   from the city's buildings, in a local forest (pines, oaks, garrigue) rather than a jungle.

**Decided:**
- **Mission maps drive the core** from here. D-059's city sandbox stands as the integration
  target and keeps its content rules; its step order waits on the maps' technologies.
- **Repositories:** the engine public; the game private (a repository the owner creates, under
  the city sandbox's rules: never published, nothing bought or downloaded without the owner's
  go). Until it exists, the map's generic parts live in `demos/`, nothing licensed.
- **The check:** `tools/verify.sh` keeps the gate on every push; a map check (one seed, fixed
  views, a walked path, an accepted set) is the check per commit for the map's demo; Tier 1
  and 2 run at milestones on the owner's word. D-043 is amended so.
- **Parallel sessions:** off the GPU only, each on its own crates, three at most; research one
  agent at a time.
- **Rendering:** RT-first on the RTX 5070 Ti (D-008's requirement stands). New rendering work
  targets DLSS 4's features through Streamline (super resolution, ray reconstruction, frame
  generation with hardware flip metering), a neural radiance cache, hybrid ray tracing and path
  tracing when the quality asks, each flagged for patents as the research rule wants. No work
  on lesser methods: a new feature needs no non-RT fallback. Cross-vendor Vulkan code stays
  the rule (no subgroup size assumed, limits read from the device), and the AMD equivalents
  (FSR 4) come when they can be tested.
- **The engine's long goal** (the owner's list): environments, biomes, planets and solar
  systems, weathers, fluids, structures and civilisations in many styles, beautiful and
  physically accurate, with loading time and performance tracked. Each map scenario exercises
  one slice of it.

**First steps:**
1. Finish the night's engine fixes on #264 (parapets mitred; the far walls' LOD weighs the
   texture seams).
2. #266, the first map: a generated Mediterranean village in its forest, entered on foot,
   with its map check; the geometry and LOD faults from the sandbox's tracker fixed there
   first; before the village is laid out, a research note on the lots' shapes and the
   joined-building types (the owner, the same night: non-rectangular buildings for no reason,
   slivers and wedges between them; semi-detached, terraced, row, attached).
3. The owner creates the private game repository; the city sandbox's `docs/TRACKER.md` keeps
   the city's twenty sightings for short sessions.
   The parallel sessions off the GPU, each on one issue (the owner: "start the parallel
   tasks", the same night): #267 sound (`forge-audio`, headless), #268 networking
   (`forge-net`, headless), #269 flora and fauna (growth, placement and behaviour as data,
   headless).
4. The empty buildings: list what the kit and the structure draw inside a closed mass (floors,
   cores, the faked rooms) and cull it until entered; a suspect for the triangles past the
   roofs at the coarser levels of detail.
5. Research to refresh for the maps: structures with interiors (D-060 continues on the maps),
   trim sheets (`research/trim-sheets.md`), flora (`research/planet-environment.md`), DLSS 4's
   features and the neural radiance cache (patents flagged). A resource the owner named for
   physics and interstellar travel: Kitten Space Agency (in development,
   `https://ahwoo.com/app/100000/kitten-space-agency`).

## D-063 — Flora as data: species grown into branch graphs, placed by their habitats region by region ✅ (proposed 2026-10-16, #269; accepted on the recommendations by the owner the same day: "Follow recommendations for now, this patent is strange as well"; the growth follows D-013 ✅)

**The question:** D-062's parallel session on the flora's simulation side (#269): how the
village's trees and shrubs (#266) are grown, where they stand, and how the city's trees
(`forge-procgen::city::trees`, #257) join the same system. Research: `research/flora.md`.

**Built (within D-013 and the issue's scope):**
- **Growth** (`forge-flora::grow`): space colonisation (Runions et al. 2007) for each bud's free
  space, the self-organising vigour of Pałubicki et al. 2009 with Borchert and Honda's apical
  control, the pipe model's radii, a Chapman–Richards curve for the envelope by age; pruning and a
  prevailing wind as inputs. No patent found on either method.
- **Species as data:** one TOML record each (`data/mediterranean.toml`): the published height and
  spread with their sources, the crown's shape, a dozen growth parameters marked as estimates, and
  a habitat (bands of soil, moisture, heat and slope, the light it needs, its spacing and opacity).
  Since #296 also how it is drawn (`[species.look]`: its leaf atlas's spray, two tones of bark and
  of leaves) and a planted tree's lifted crown (`planted_lift`, a share of its height).
- **Meshed for the renderer** (#296, from city-plan's forest): `forge_flora::grow::individual`
  gives a forest's individuals (three ages of three seeds), `forge_geom::flora::grow_plant` grows
  one and meshes it: its wood, its crown's lumps and leaf cards (`grown_mesh`), and the opaque proxy
  the probes' rays meet (`grown_proxy`, #297). The village's images and cooked meshes are the same
  to the bit as before the move.
- **Placement** (`forge-flora::place`): the ground's fields (slope, the sun's heat by aspect, soil
  depth from the relief and the flow, moisture from the drained area), planted rows and groves
  first, then the wild canopy, then the understorey in the light it leaves, each by darts in
  priority order with spacing by pairs of species; a pure function of the ground, the rules and
  the seed.
- **Measured** (`docs/demos/flora.md`): a grown plant in 4 ms (cistus) to 63 ms (holm oak, 8 405
  nodes); a square kilometre placed in about 1.1 s on one thread (266 143 plants); 17 tests.

**Accepted (the owner, 2026-10-16; they find the patent below strange too):**
1. **Region by region, never around the player.** Placement runs once per region (a map, a
   world cell) and is cached or recomputed identically. Sony Online's US 8,115,765 B2 (now
   Daybreak, active to about 2028-01-07) claims flora generated by rules in a user-defined area
   around an avatar and updated as it moves; a streamer of that shape waits for the owner's word
   or for 2028.
2. **The city's trees join it** through one small hook in `city::trees` (the coordinating session
   makes it): its forest grid replaced by `forge_flora::place::place` over the site's heights with
   the city's water, districts, roads and blocks as `keep_out`, and its street, park, quay and
   courtyard trees given a species each (`Rule::Row` and `Rule::Grove` are those settings).
   The street species (plane, lime, umbrella pine of #257) come with their own research.
3. **A few individuals per species and age class** grown once into D-053's derived cache and
   instanced by the placement, rather than one graph per placed plant.

*Not chosen:* L-systems (a fixed topology that neither the light nor the neighbours shape);
Weber–Penn's recursive levels as the generator (kept only as the idea of a flat species file);
shadow propagation within one plant (the markers stand for both space and light at this scale).

## D-064 — Particles: the splashes grown into emitters as data, lit per particle, textures baked by our own solver ✅ (proposed 2026-10-16, D-062's next core step after the flora; the owner's go 2026-10-17)

**The question:** how the mission maps' effects are made (a chimney's smoke, dust under the
steps, falling leaves; later sparks, fire and rain). Research: `research/particles.md`.

**Proposed:**
1. **Emitters as data** in `forge-render`, grown from the splashes (#107): a TOML record each
   (spawn, life, size and colour over life, forces: gravity, drag, wind, curl noise; collision;
   how it is drawn), simulated on the async compute queue from deterministic streams (each
   particle's birth time and random numbers from its emitter's seed and index), so captures stay
   exact.
2. **Collisions** against the depth buffer, D-034's depth from above, and the TLAS for sparks and
   debris.
3. **Drawing:** soft sprites, velocity streaks, flipbooks blended by motion vectors, six-way lit
   smoke; additive effects unsorted, alpha-blended ones sorted per emitter (weighted blended OIT
   where emitters overlap heavily); smoke and dust also written into the dust's froxel volume
   (D-032), which casts their shadows.
4. **Lighting per particle:** one sun ray a particle a frame against the TLAS, the GI probes at its
   centre. Particles are not put in the TLAS.
5. **Upscalers:** the particles' pixels in the reactive mask TAA reads, passed to DLSS as its
   transparency mask.
6. **Textures baked by our own offline solver:** a small stable-fluids smoke solver (Stam 1999;
   US 6,266,071 expired in 2018), rendered from six light directions with its velocities as the
   motion vectors. EmberGen (a purchase) or downloaded sets only on the owner's go.
7. **First effects:** chimney smoke at dusk, footstep dust (D-012's foot-down events), falling
   leaves; each in the F1 overlay and `PROFILE.md`, with the still-camera and dolly shimmer checks.

**Patents:** none found on GPU particle simulation or sorting, six-way lighting or weighted blended
OIT. Pixelworks' US 11,941,744 B2 (motion vectors for frame interpolation, active to 2042) does not
cover flipbooks, and nothing proposed uses it.

**For the owner to decide:** the scope above, and where the textures come from (our own solver,
recommended; or EmberGen; or downloads).

*Not chosen:* particles in the TLAS (their reflections are rarely seen, their cost grows with each
particle); a shadow ray per pixel (one a particle suits sprites); per-pixel linked lists or
moment-based OIT (memory and bandwidth the village's few emitters do not need); smoke simulated
at run time (a hero effect later).

**Amended (2026-11-07): puffs lit by the lamps.** An effect may take a share of the lamps' light (`Effect::lamps`, 0 by default), where a scene asks (`Look::particle_lamps`, `Particles::light_by_lamps`): the pass `particles/lamps` (`shaders/particles_lamps.slang`) weighs the frame's lamps at each puff's middle, unshadowed, as a froxel of the fog (D-099) does, once a puff a frame. It is its own file because it needs `meshlet.slang`, so other scenes neither compile nor run it. The `haze-room*` effects are a room's hanging smoke. On the private district's bar, fog alone had stood in for that haze, and one lamp's column of it read 9 times the source's wall. With about 400 puffs: `particles/draw` +0.04 ms, the path +0.1 ms.

**Decided** (the owner, 2026-10-17): "For D-064 go with recommended". The scope as proposed (the
seven points above) and the textures baked by our own stable-fluids solver; EmberGen or
downloaded sets stay out unless the owner says otherwise.

**Built so far (step 1, 2026-10-18, #271):** `forge_render::particles` and
`shaders/particles.slang`, shown by the physics lab's `--lab smoke` (`docs/demos/physics-lab.md`).
- **Point 1:** emitters as data (`data/particles.toml`: chimney smoke, footstep dust, embers),
  streams and bursts, deterministic births and random numbers. They are simulated on the async
  compute queue: gravity or buoyancy, a drag towards the wind, a curl-noise swirl.
- **Point 3, in part:** soft puffs, eroded by noise and shaded as lumpy balls against the sun.
  Smoke and glow share one premultiplied blend, so `VertexPipelineDesc` gained `premultiplied`.
- **Point 4:** one sun ray a particle a frame, and the GI probes' light at its centre from above
  and below (the open sky's beyond them; the probes added the same night).
- **Point 5, in part:** TAA's reactive mask.
- **Point 7, in part:** the three first effects, in the lab.
- **Point 6 and point 3's six-way smoke, the same night:** `forge_render::smoke_bake`.
  - A puff on a 64³ grid by Stam's stable fluids, grown by its heat's expansion (a divergence
    source), its curls kept by vorticity confinement.
  - 32 frames, each lit from the six axis directions, in Unity's six-way layout.
  - About 2 s once, then the derived cache's (1 ms).
  - The chimney smoke draws from it: frames cross-faded by age, turned and mirrored per
    particle, the six lights weighted by the sun's direction. The dust keeps procedural puffs.
- **Numbers:** 492 particles at most, the draw 0.085 ms with the flipbook (0.022 ms
  procedural), 0.006 ms of simulation. Two runs are identical to the pixel.
- **Point 2's TLAS part and point 3's streaks, the same night:** an effect with a `bounce` meets
  the scene, and one with `streak` is drawn along its flight. The lab's grinder sparks have both.
  - **The step:** each step of a particle that meets the scene is a ray against the static
    structure. Where it hits, the particle stops 1 cm short and its velocity bounces off the
    surface: its speed into it times the bounce, the share along it slowed by 30 %.
  - **The normal:** a ray query gives no triangle's normal without its vertices, so two more
    rays 2 cm beside the first find the surface's plane, only where it hits.
  - **What it does not meet yet:** the movers; and it takes every surface as opaque.
  - **The streak:** from where it was 1/50 s ago to where it is, a pixel wide at least, its light
    spread along it.
  - **Cost:** 832 particles at most, `particles/advance` 0.005 → 0.018 ms.
  - The depth buffer's collisions (for smoke and dust) and D-034's depth from above are still
    to come.
- **The fire's flipbook (point 6), the same night:** `smoke_bake::bake_fire`. The smoke's puff,
  its heat drawn as a flame's glow.
  - The colour runs from deep red to orange to yellow-white with the heat, and the brightness
    with its fourth power. The soot is thinner than the smoke's.
  - It is one atlas of the same 32 frames, light stored as its square root.
  - An effect with `texture = "fire"` draws a glow from it, cross-faded and turned as the smoke
    is. The lab's fire has flames.
  - The fire does not light the yard: as a lamp of the flames' size, the rocks' shadows came
    out dithered at their wide penumbrae, which wait for the lamps' denoiser (D-065).
- **Point 3's frames blended by motion vectors, tried and left out (2026-10-18):**
  - The bake wrote each frame's motion to the next: the velocity along each column, weighted
    as the render weighs the smoke, in a third atlas.
  - The draw pulled both frames along it.
  - With 32 frames over the puff's life, a frame's content moves about a texel to the next
    (±0.012 of the frame). At frame 360 of the smoke yard the warp changed 37 pixels of 1.44 M
    (over 2 codes), so it is not in the tree.
  - It is worth again for a flipbook with few frames for its motion.
- **Point 3's sorting, tried and set aside (2026-10-18):**
  - The method: a bitonic sort of the live particles far to near on the async queue, keyed by
    distance. A workgroup sorts 1 024 pairs in shared memory, and global steps take the larger
    stages.
  - It ran deterministic: two runs and both paths identical.
  - It changed 4.1 % of the smoke yard's pixels, for the worse. The fire's oldest puffs (2 m
    across, their late frames diffuse) drift towards the camera and now cover the young
    detailed ones, which birth order drew on top: a dark blur in front.
  - The smoke's sizes and fades were tuned under birth order. Sorting waits for a re-tune of
    them, judged by the owner's eye: smaller old puffs, an opacity that falls faster with age.
- **Next:** the depth buffer's collisions, sorting with the smoke re-tuned, a dust flipbook, the
  dust volume, and the village's chimneys and steps.
- **Point 5's DLSS half, tried and left out (2026-10-18):** Streamline 2.14.16's DLSS reads none
  of the hints.
  - The reactive mask was tagged in turn as the bias-current-colour hint (type 29), the
    reactive-mask hint (36) and the transparency hint (27), copied to `R32_SFLOAT` (Streamline
    refused TAA's `R8_UNORM`: "Cannot have undefined format").
  - The smoke yard under `--dlaa` came out bit-identical with and without each one.
  - DLAA keeps its own handling of the particles. The attempt is not in the tree.
- **A blood spray** (2026-10-19, asked by an out-of-tree town for its rifle's hits): `blood-spray`
  in `data/particles.toml`.
  - A burst of 36 dark red droplets, lit puffs 3–4.5 cm in radius, launched at 1.5–3.5 m/s within
    35° of the shot.
  - They fall under full gravity and stay where they meet the scene (a bounce of 0) until they
    fade, in 0.6–1.2 s.
  - Looked at in the smoke yard, a burst in place of a step's dust: they read as dark drops in
    the air and on the ground.
  - Appended last, so the other effects keep their indices: the smoke yard's captures are at
    0 pixels.
  - A spatter that stays waits for decals (#304).
- **Steam** (2026-11-05, for the private district's steam, D-083 step 4): `steam-column` and
  `steam-vent` in `data/particles.toml`, appended after the blood spray so the other effects
  keep their indices. (The numbers below are the first tuning's; the columns and vents were
  retuned on 2026-11-05: see "The steam's look" in this entry's amendment.)
  - **A column:** a sewer grate's, a manhole's, a tall vent's plume. Four puffs a second, lit as
    the smoke, procedural (no flipbook). Each is 0.9 m in radius at birth and swells to 3.2 m over 4–6.5 s, 7 % opaque at
    birth. Together they read as one body of haze, and the wind takes it.
    - They rise at about 0.65 m/s (the lift against the drag), their middles 2.5–5 m. The
      source's tallest plumes (20 m) come out lower; an emitter has no scale, so a taller one
      would be its own effect.
  - **A vent:** an air conditioner's, a pipe's, a kitchen's. Eight short wisps a second over
    1.5–3 s, 0.35 to 1.6 m in radius and 6 % opaque.
  - **Tuned by eye** against the source's frames, not physically: the column is born over a
    disc 2.4 m wide and takes 0.6 of the wind, and the vents' last wisps are wide but faint.
  - **First tuning, left out:** fewer, denser puffs (12 % opaque, up to 2.2 m in radius). Beside the
    source's frames they read as a stack of balls, each puff's rim lit by the sun. Fainter, wider
    puffs blend.
  - **The baked flipbook, left out:** with the smoke's flipbook, a column a few metres from the
    camera along the source's path drew a coil of lit rims, one per puff. A still view hid it.
    Procedural puffs have no rim and read as one soft shaft, as the source's do.
  - **In the district:** its viewer places the source's steam emitters and emits those within
    60 m of the camera. The cost is in `docs/PROFILE.md`.

**The steam near the camera, under DLSS and in the sun (2026-11-05).** The owner, on the district's
street: "outside is not great either", "the fog "pops"", and on a sunlit manhole's column, "this
one pops a lot", "same for other smoke/fog emitters outside".
- **The camera in a puff:** a puff the camera walked into covered the screen at full opacity. It
  vanished in one frame when its middle passed the near plane, and the street went from orange
  haze to clear.
  - **The fix:** each particle fades out as the camera comes within two of its radii, and is
    gone at half of one (`smoothstep`, a camera depth fade as Unreal's).
  - **The result:** the street's largest one-frame step in the frame's light went from 30.4 % to
    7.1 % (4.3 % without steam, the camera's own motion).
- **The sun on a puff:** one shadow ray from its middle turned a puff metres wide from lit to
  shaded in a frame as it crossed a shadow's edge, and a sunlit street's steam flickered.
  - **The fix:** five rays, from its middle and from four points half its radius out across the
    sun's way (turned by its seed), averaged and eased over 0.15 s (`sun_share`, `SUN_EASE`). A
    point a surface stands between (the ground under a grate's steam, the wall a puff leans on)
    is left out: its ray would start inside the surface.
  - **The result:** over six seconds of the street under DLSS Quality, the 4 × 6 tiles' sudden
    steps in light (over four times their neighbours') went from 162 to 51; those left are in
    dark tiles, the camera's motion. `particles/advance` stays at 0.001 ms.
- **The blocks:** under DLSS (Quality and DLAA) the steam showed square blocks and a fine grid.
  It did not with the anti-aliasing off or with FSR, which takes the particles' reactive mask.
  - DLSS takes no reactive mask: NVIDIA's guide (3.15) says the current models ignore it.
  - DLSS held the fading steam in its history. In the steam's region at the same frame: 0.225
    under DLSS, 0.178 with the anti-aliasing off.
  - **Tried and left out:** the puffs' own motion vectors (their middles moved by their velocity
    and the camera) written where they cover a third of a pixel, then a fiftieth. The blocks
    stayed. A translucent layer over a surface has no one motion.
  - **Tried and left out:** the particles drawn after DLSS, over its frame, at the window's size
    and without the jitter. The steam was clean, but the frame took 0.45 to 1.05 ms longer and
    unsteadily (p99 18 → 20 ms on the district's path). The draw that reads the particles' ring
    moved to the frame's end, and `fog/light` lost the draw it had run beside.
- **A layer of their own** (`ParticleParams::after_upscale`, `Particles::composite_after_upscale`;
  `--particles-before-dlss` the A/B):
  - **The order:** under DLSS the particles are drawn where they were, at the render's size and
    with its jitter, but into a layer of their own: their light, and the share of each pixel
    they cover blended over. `particles/composite` then lays the layer over DLSS's frame: read
    where each pixel's point lay in the jittered render, bilinear, over what is there by the
    share covered.
  - **The result:** the steam follows the steam without anti-aliasing (0.182 against 0.178 at
    the same frame), with no blocks. It is a little softer, upsampled from the render's size.
  - **The cost:** `particles/composite` 0.04 ms at 2560 × 1440; on the district's path (DLSS
    Quality, interleaved runs) the frame 15.77–15.80 ms in the old order and 15.82–16.07 ms with
    the layer, p99 17.75–18.10 both. The transient heap's peak is the same (134 MB): the layer
    takes memory free at that point of the frame.
  - **A still view** of the street's steam (2560 × 1440, frames ten apart): pixels changing by more
    than 2 codes 12.9 → 17.0 % (at the path's 53 s) and 9.8 → 13.2 % (51 s). The change lies in
    the moving steam, which DLSS no longer averages. Edges outside the steam change no more
    than before.
  - **What the order changes:** the exposure's meter, which reads the frame before the upscale,
    no longer sees the particles; the bloom and the local exposure still do. The froxel fog
    (D-099) no longer covers them, and they keep their own aerial haze. Before, the fog was laid
    over them at the depth of the surface behind, too much for a near puff.
  - **FSR, TAA, no anti-aliasing, and a scene with the tank** (#156, whose water refracts the
    scene with the particles in it) keep the old order and the reactive mask.
  - **Patents:** none found in a search (2026-11-05). Epic's guide to temporal upscalers moves
    translucency's pass for the same reason.
- **The steam's look** (the same day). The owner's still of the manhole's column was a bright
  ball standing out of its shaded street. Beside the source's frames:
  - **The source's columns:** grey in the shade and only a little brighter in the sun, and
    narrower and taller (frames 28 and 37).
  - **Ours:** puffs swelling to 3.2 m made a ball, and the low sun lit it orange-white.
  - **Opacity alone, tried:** 4.5 % and 3 % faded the shade's steam away and left the sunlit
    shaft the brightest thing in the alley.
  - **The fix:**
    - **A per-effect share of the sun's light** (`Effect::sun`, `sun` in `particles.toml`, 1 by
      default; the sky's light stays whole). The lab's smoke is unchanged to the pixel.
    - **The columns:** a fifth of the sun, 10 % opaque at birth, puffs up to 1.8 m, rising
      1.0–1.8 m/s, six a second. The vents take a fifth of the sun too, 8.6 % opaque.
  - **The 14 frames:** geometric mean +0.12 → +0.08 EV, RMS 0.77 → 0.75 (0.63 without steam).
    `particles/draw` on the path 0.23 → 0.135 ms, the narrower puffs covering fewer pixels.
  - **Sizes, by the source's names** (the same evening): two more effects, appended so the others
    keep their indices.
    - **`steam-column-small`:** wisps up to 0.8 m, for steam a metre wide or less, a sewer lid's
      holes and grates (`*_0p5x2m`, the 89 `*_sewer_spouting_4_holes`, `*_sewer_grate`). Drawn
      as the column, they laid frame 37's band across the street.
    - **`steam-plume`:** puffs up to 4 m rising about 2 m/s for 6–10 s, for the 10–65 m
      plumes. The district's viewer emits them out to 200 m (the rest to 60 m).
    - **The result:** the 14 frames' rms 0.57 → 0.56. `particles/draw` on the path 0.135 →
      0.107 ms, with 3,846 particles alive at most (1,867).
    - **Left open:** the source's 67 kinds of steam come down to four effects.

## D-065 — Lighting the insides: lamps by clustered lists until ReSTIR's patents are settled ✅ (proposed 2026-10-16, #270; the owner's go 2026-10-17: both, compared)

**The question:** how the village's entered houses are lit (#270): lamps, the rooms' bounce
light, windows seen from inside. Research: `research/interior-lighting.md`.

**The patent question first.** D-008 (✅ 2026-09-24) chose ReSTIR for direct light in the
ray-traced tiers, before the owner's rule of 2026-10-10. NVIDIA holds US 11,315,310 B2 on reservoir
resampling with a global-illumination structure (active to about 2041, with later filings in its
family); whether a patent covers ReSTIR's direct-light method alone was not settled. Building on
ReSTIR waits on the owner's word.

**Proposed:**
1. **Lamps by clustered light lists** (Olsson et al. 2012, no patent found): each pixel picks one of
   its cluster's lights by its unshadowed contribution and traces one shadow ray; accumulated in
   time and denoised (NRD's diffuse denoiser, to integrate). A village's room has one to four
   lamps, so a pixel has a handful of candidates.
2. **Lamps as data:** a ceiling lamp a room and one by each bed and sofa (`city::furnish`), in
   candelas and kelvin, lit from the households' routines as the faked windows are.
3. **Probes for rooms:** a finer finest cascade on walked-in maps (2 m, or 1 m round the walker, as
   the models lab's rooms), measured against today's 4 m; the probes' hits lit by the lamps.
   *Measured the same evening:* at 2 m and 1 m a hall lit through its door stayed as dark; the
   cause was the street's fixed exposure, and the village now meters (`Look::metered`,
   `demos/city-plan.md`, "Open at every opening, and metered").
4. **Glass both ways** for the near houses with drawn insides; the far ones keep their faked rooms.
5. **Checks:** a night time in the map check's inside views, the shimmer scripts, the F1 overlay
   and `PROFILE.md`.

**For the owner to decide:** whether to take ReSTIR (D-008) despite NVIDIA's patents, or the
clustered lists proposed here; and the order of the four parts.

*Not chosen for now:* a light tree sampled stochastically (Moreau et al. 2019: thousands of lights
are more than a village needs, its patents not searched); lamps as emissive surfaces only (no
shadows, no light on the walls).

**Decided** (the owner, 2026-10-17), after the patent question was set out (a patent covers the
method whoever writes the code; RTXDI's licence, the NVIDIA RTX SDKs licence of 2025-01-31, has
no express patent grant, requires applications to work with NVIDIA GPUs, not only, and forbids
making the SDK subject to an open-source licence). The owner: "if we use the sdk or available
libraries as long as the license allows it we can use them. But we should absolutely not copy
the code made by them that is patented", then "for D-065 I don't know which of 1 or 2 is better,
maybe try both ? And compare results with me ?"
- Both are built and compared with the owner: (1) the clustered light lists proposed here, and
  (2) NVIDIA's RTXDI (ReSTIR DI) used as the SDK, under its licence, fetched into an ignored
  directory as the NRD and Streamline SDKs are, never copied into Forge's code.
- The other parts (lamps as data, probes lit by the lamps, glass both ways, the checks) serve
  both.

**Built so far (step 1, 2026-10-18):** `forge_render::lamps` and `lamps_at` in `meshlet.slang`,
shown by the physics lab's `--lab lamps` (the sharpness room at midnight, three lamps).
- **The lamps:** small sphere lights in candelas and kelvin (Tanner Helland's blackbody fit,
  scaled to luminance 1).
  - `MeshletScene::set_lamps` keeps, each frame, the 64 nearest whose light reaches the camera's
    surroundings.
  - A lamp's reach is where its light falls to 0.05 lux.
- **The shading:** every pixel takes every lamp whose reach it is in.
  - Its illuminance is the candelas over the distance squared, by the cosine, let go smoothly at
    the reach.
  - One shadow ray goes towards a point of its sphere that moves with the frame's noise, so TAA
    (or DLAA) averages the penumbra.
  - It gets highlights as the sun's. `sun_light` adds both, so every class takes them.
- **Not the clustered lists yet:** a pixel loops over the frame's few lamps. Olsson's clusters,
  one ray a pixel to a lamp picked by its contribution with the denoising, RTXDI to compare, and
  the probes lit by the lamps come next.
- **Checks:**
  - two runs at once identical, the mesh path matching the fallback;
  - a still camera's frames 10 apart differ on 0.21 % of the pixels by more than 2 codes under
    TAA and 0.0094 % under DLAA (the room unlit: 0.0102 %);
  - the cost: three lamps over the whole view, three rays a pixel, take `shading/standard` from
    0.16 ms to 0.48 ms.

**Built next (steps 2 and 3, 2026-10-18):** the village's houses get their lamps, the rooms are
the light lists, and the probes take the lamps' light. Shown by `city-plan --village 600` at
night (`--time-of-day 1.333`, 22:00), and in the lab's room.
- **Lamps as data** (`city::furnish::lamps`, part 2): 350 rooms, 134 hall storeys and 162 beds
  and sofas give 646 lamps.
  - A frosted globe hangs over each room's middle, as bright as the room is large: 70 lm a
    square metre, from 450 to 1 600 lm, a third more and whiter in kitchens and bathrooms.
  - The hall has a light on its ceiling on each storey, clear of the flight.
  - A sconce hangs over each bed's head and each sofa's back that stands against a wall.
  - The households light them by the faked windows' rule (`Residents::lit`): 325 are on at
    22:00, 121 at 15:00. The globes glow where they are on (`furniture::lamp`), and the probes'
    rays take none of their glow (`RenderLayer::lamp`): the lamp's light already counts.
- **The rooms as the light lists** (part 1, in place of Olsson's clusters for the houses):
  - Each lamp names the probes' regions it lights (D-068): its room's, and those across its
    storey's doorways, not the street's.
  - Each frame, the lamps' (region, lamp) pairs are sorted by region. A pixel finds its region
    (`region_at`, as the probes do) and binary-searches its run.
  - A house lamp lights only its storey, from 2.7 m under it to 0.8 m over it. The hall is one
    region through every storey, and the bedrooms upstairs, behind the floors, took its ground
    floor's places.
  - Looping over the frame's 64 lamps cost 0.5 ms in a hall. Olsson's clusters stay for lamps
    outside the houses (the streets' lamps).
- **At most five rays a pixel:** the three lamps with the most unshadowed light there get a
  shadow ray each, the strongest two, and one of the others is picked by its light (Shirley,
  Wang and Zimmerman 1996) and counts as its light over its chance. Every lamp counts, and TAA
  averages the others' noise.
  - The lamps' sphere points are the sun's eight Vogel points (#54). Independent points each
    frame left streaks across a doorway's penumbra.
  - The strongest lamp's two rays go to points half the cycle apart, so TAA sees all eight in
    four frames. That took the hall's flicker under TAA from 1.0 % to 0.65 %.
- **The probes lit by the lamps** (part 3): each probe ray's hit takes one lamp of the probe's
  region, picked by its light, with one shadow ray. A room's walls send its lamps' light back,
  one bounce a frame. The mirror rays' hits take them too.
- **A white balance for the lamps:** their colours are as a camera set for 3 200 K sees them
  (`lamps::WHITE_BALANCE`). At the sun's white point a 2 700 K bulb turned the white rooms deep
  orange; now it stays warm. The sun and the sky keep their colours.
- **Checks:**
  - the hall at 22:00 with a still camera: frames 10 apart differ on 0.004 % of the pixels under
    DLAA (0.0035 % without the lamps' light, `--no-lamps`) and 0.65 % under TAA (0.12 %);
  - in motion (the room turning 4° a second, DLAA against a 2 × 2 supersampled run), 1.65 % of
    the pixels change otherwise than the reference's, against 2.36 % without the lamps' light;
  - the cost in that hall, best of three: `shading/standard` 0.47 → 0.96 ms and `gi/probe rays`
    2.17 → 2.25 ms; the frame 4.29 → 4.86 ms.

**The penumbra filter (2026-10-18):** the interleaved ratio filter that
`research/lamp-penumbra-denoising.md` §7 recommends (no patent found), in `meshlet.slang`'s
`lamp_rays_main` and `lamp_filter_main`. A filter whose width comes from the occluder's distance
is in NVIDIA's US 10,740,954 claims 10 and 15, to about 2039: the filter's size is fixed. Wide
penumbrae no longer come out dithered and stair-stepped, and the smoke yard's fire lights the yard
again (`demos/physics-lab.md`).
- **The structure: the rays before the resolve.** Two graph passes after the visibility buffer,
  only when the frame has lamps (no other capture changes):
  - **"shadow/lamp rays"** rebuilds each pixel's surface as the sun's SIGMA pass does
    (`pixel_surface`, shared with it). It takes the lamps the resolve took (`lamp_picks`: the
    three strongest, the strongest with two rays, and one picked by its light) and traces their
    rays. It writes Ŝ / Û per channel, the same rays' shadowed over unshadowed irradiance
    (Heitz, Hill and McGuire 2018), with luminance(Û), and the surface's position, normal and
    region.
  - **"shadow/lamp filter"** averages that share over its neighbours.
  - **The resolve** lights every lamp of the pixel's place exactly, unshadowed and without a
    ray, diffuse and highlights, times the filtered share. The other way, keeping the rays in the
    resolve and correcting the colour before TAA, needed the lamps' share of each class's colour,
    which the classes mix differently.
- **The disc points:** a table of 128, the first 128 points of the 2D Sobol sequence (1967)
  through Shirley and Chiu's concentric map (1997). Point 16 (frame mod 8) + B[x mod 4, y mod 4],
  B a Bayer matrix (Keller and Heidrich's interleaved sampling, 2001), so every 4 × 4 window
  holds a whole stratified run of 16 each frame, and TAA's 8 frames all 128. Each lamp keeps its
  golden-angle turn; the per-pixel turn is gone.
- **Two changes to the recipe, each measured:**
  - **A 7 × 7 tent, not a 4 × 4 box.** The box left each disc point's shadow edge on its own
    4-pixel grid. The rocks' penumbrae showed 4-pixel steps, and TAA's and DLAA's clamps kept them
    (the frame-to-frame changes are smooth in space, so the clamp's box is narrow). The tent is
    the box twice over (weights 1 2 3 4 3 2 1 each way), so each column and row of a 4 × 4 block
    still weighs a quarter: every point of the run counts the same. Its edge weights: the same
    region (a room holds its own lamps), n·n′ > 0.9, and within 0.5 % of the distance (and 1 cm)
    of the pixel's plane.
  - **The strongest lamp's second ray goes to the run paired with the frame's** (runs 2m and
    2m + 1), not half the cycle away: its 32 points a frame are an aligned run of the sequence,
    stratified as a whole. A still camera's frames 10 apart changed on 6.1 % of the smoke yard's
    pixels under TAA with frame + 4, 3.9 % with the pair (the hall 0.31 % and 0.16 %).
  - *Tried and left out:* the Bayer index xored with the frame's run (a pixel's points cross the
    disc over the cycle): no better on any check once the tent was in.
- **Checks** (1600 × 900; a still camera's frames 10 apart from 200 to 280, the pixels that
  change by more than 2 codes):

  | | before (the resolve's rays) | the filter | without the lamps' light |
  |---|---|---|---|
  | the hall at 22:00, TAA | 0.63 % | 0.16 % | 0.12 % |
  | the hall at 22:00, DLAA | 0.0027 % | 0.0024 % | 0.0030 % |
  | the smoke yard's fire at midnight, TAA | 10.9 % | 3.9 % | 0.73 % |
  | the smoke yard's fire at midnight, DLAA | 1.44 % | 0.034 % | |
  | the lab's lamp room (bulbs of 4–6 cm), TAA | 0.078 % | 0.081 % | |
  | the lab's lamp room, DLAA | 0.0041 % | 0.0040 % | |

  - The smoke yard's clock is held (`--sea-time 6`), so the fire's flicker and the particles
    stand still. What still changes under TAA is the soft shadows from frame to frame: with every
    ray aimed at the fire's middle, 0.56 %.
  - **In motion** (the hall turning 4° a second, DLAA; each 10-frame step against the same run
    supersampled 2 × 2, `imgdiff --then`): 3.80 % of the pixels change otherwise than the
    reference's before, 1.34 % with the filter, 1.29 % without the lamps' light.
  - **Determinism:** two runs one after the other, the mesh path against the fallback and the
    serial frame (`FORGE_ASYNC=0`) give the same pixels, in the hall and the smoke yard, and two
    runs at once in the hall. Two runs at once in the smoke yard part on one pixel by one code in
    about half the pairs: the filter's light puts that pixel on a rounding edge. It was TAA's
    display pass, precise already, evaluating its `lerp` (`FMix`, which precise mode does not
    reach) two ways; since #281 the display path mixes with `lerp_exact`, and 30 pairs at once
    agree (`docs/PROCESS.md`, "`lerp` escapes precise mode").
  - `--no-lamp-filter` keeps the resolve's rays: its images are the base's to the pixel.
  - **Cost** (best of three, `docs/PROFILE.md`): in the hall, where every pixel takes up to four
    lamps, "shadow/lamp rays" 0.64 ms and "shadow/lamp filter" 0.16 ms, and `shading/standard`
    falls from 0.95 to 0.65 ms. The frame goes 4.88 → 5.33 ms (serial, `FORGE_ASYNC=0`: 5.03 →
    5.49 ms), +0.45 ms, more than the note's 0.1–0.2 ms:
    - the rays pass spends 0.19 ms before its first ray (the surface again, its region, the
      picks);
    - the rays take 0.45 ms in a pass of their own, where the resolve's other work no longer hides
      them;
    - the tent costs twice the box (0.16 against 0.08 ms).

    The lab's lamp room, three lamps over the whole view: +0.18 ms (1.84 → 2.02 ms). The smoke
    yard's fire: +0.08 ms in its zones. A frame without lamps runs neither pass.
- **Next:**
  - RTXDI: compared, and the street lamps by it behind `--street-restir` (below, after the glass).
  - If wide penumbrae still crawl under TAA (the fire's floor), the note's upgrades: an à-trous
    pass on the windows that mix lit and shadowed pixels, or SIGMA's translucency route fed with
    the same ratio (it needs closest-hit rays and the owner's word, given §3's patents).
  - The light through windows onto the street.
  - The streets' lamps: lit since 2026-10-18 (below, after the glass). The far ones are lit by
    ReSTIR behind `--street-restir` until a denoiser; Olsson's clusters stay the other way, after
    a look at AMD's Forward+ patents (US 10,803,655 B2).
  - Glass both ways (part 4). From the street, the near houses' windows still show the faked
    rooms, though their real rooms are lit. Two ways, for the owner to choose (2026-10-18):
    - **(a) Traced through the glass in the resolve.** The near houses' glass rows get a flag,
      and a glass pixel traces one ray into the room behind and shades the hit as a mirror ray's
      (sun, probes, one picked lamp), blended with the glass's own reflection by Fresnel.
      - Cheap: one or two rays a window pixel.
      - The room is shaded flat (its rows' average colour, no AO).
      - TAA reprojects it with the glass's motion: it smears as the camera moves past, unless
        it gets a history of its own as the mirror rays have.
    - **(b) The glass drawn after the resolve.** The near houses' glass leaves the visibility
      pass and is drawn over the resolved room behind it, blended as the particles are, with its
      reflection and tint.
      - The room is drawn at full quality and has its own motion vectors.
      - It needs a pass for transparent meshes (their cull and draw), and the kit's near windows
        on rows of their own. Several sessions.
    - Recommended: (b) for the look; (a) only as a stopgap with its own history.
    - **The owner chose (b), the glass pass (2026-10-18).**
    - **Built the same day** (`forge_render::Glass`, `shaders/glass.slang`):
      - **The visibility pass leaves the glass out.** The houses whose insides are drawn take
        the kit's groups again (`plan-kit-open-*`), with their glass row
        `RenderLayer::drawn_after`. Its clusters go to the cut-outs' raster, which drops every
        fragment of it, so the resolve shades the real room behind the window.
      - **The rays still meet the faked glass** (`interior`, glazed). Taking it out of the rays'
        scene as well darkened a bedroom by 2.7 EV at a fixed exposure, because D-068's probes
        were tuned with the glass there.
        - Letting the probes' rays pass the glass from every side brought back 1.4 EV of it. The
          rest is still to be found (#280).
        - "The light through windows" (below) has to look at this again.
      - **The panes are drawn after the sky's compose and the water,** in two passes. They are
        copied from the modules' glass: 1 116 triangles in the village.
        - `glass/requests`: the nearest pane at each pixel asks for its mirror ray, which is
          traced as the water's are (`trace_requested`, `glass/reflections`). From outside it
          shows the street in the windows; from inside, the room.
        - `glass/draw`: every pane is blended over what is behind it, premultiplied. The room
          shows through 92 % of what Fresnel lets pass (Schlick, F0 0.04). The sky's reflection
          and the haze are added.
        - The glass writes no depth, so TAA reprojects the room by its own depth.
      - **The far rooms' glow.** Beyond the frame's lamps (the 64 nearest), a lit room's glow
        shows on its panes, so the far houses stay lit at night: at first faded in from 30 to
        60 m at half the faked rooms' light; since #284, from the room's own lamp by the share of
        it the frame leaves unlit (below).
      - **Cost** in the map check's views: `glass/requests` 0.011–0.018 ms, `glass/draw`
        0.004–0.006 ms, `glass/reflections` 0.035–0.141 ms (the street view with the most
        windows).
      - `--no-glass` shows the rooms without their panes.
      - **Panes changed at run time** (#325, 2026-10-19), for a sash raised or a window
        smashed:
        - Each corner carries its pane's id (`GlassPanes::pane`; the village numbers a window
          piece's glass as one pane).
        - `Glass::set_pane(id, Some(transform) | None)` moves a pane from where it was given,
          or hides it with a zero transform, under which its triangles cover nothing.
        - The vertex shader applies the pane's 3 × 4 rows, so the mirror-ray requests and the
          glow follow.
        - The transforms (48 bytes a pane) are written into a frame slot's copy only on the
          frames after an edit.
        - A scene sends its edits through `Running::glass_edits`, in the world frame.
        - **Checks:**
          - With every pane at the identity, the map check's 13 images are unchanged
            (0 pixels).
          - `--glass-test` hides every other pane one second in and raises the rest 0.4 m. On
            the first house's street front, the raised window loses its glass on its lowest
            0.4 m, and the door's transom glass goes up behind the wall.
  - **RTXDI fetched (the owner's go, 2026-10-18):**
    - NVIDIA's `RTXDI-Library` 3.1 (commit f12037f, 2026-09-03) is in `rtxdi-sdk/`, ignored by git
      as `nrd-sdk/` is. It holds 68 files: HLSL includes for ReSTIR DI, GI and PT, ReGIR and the
      light presampling, and a small C++ host side. The samples, Donut and the assets were not
      fetched.
    - The licence is the NVIDIA RTX SDKs licence of 2025-01-31. Use inside an application is
      allowed, and there is no express patent grant. The SDK must not be made subject to an
      open-source licence, and modified source needs NVIDIA's notice.
    - So it stays outside Forge's code. Forge's own Slang shaders define RTXDI's application
      bridge (`RAB_*`) and include its headers from `rtxdi-sdk/Include` at compile time. The
      host side's parameters get Forge's own Rust, written from the headers' documented
      meaning, not ported.
    - It is to be compared with the light lists with the owner, now that the penumbra filter has
      landed (both touch `lamps_at`).
  - **How RTXDI plugs in** (planned from the headers, 2026-10-18):
    - **Only ReSTIR DI over local lights:** `Rtxdi/DI` and `Rtxdi/LightSampling`. No ReGIR, GI
      or PT, and no environment map.
    - **Forge's bridge** (`RAB_*`, 29 functions, a few of them stubs):
      - the surfaces come from the visibility buffer, this frame's and the last one's (a
        depth and an octahedral normal kept a frame);
      - the lights are the `GpuLamp` spheres, sampled by solid angle;
      - the target is the unshadowed light weighted by the surface's BRDF. It is zero where the
        lamp does not light the surface's region (D-068's lists as a prior);
      - the visibility is one shadow ray.
    - **Its three buffers** are named by macros (`RTXDI_LIGHT_RESERVOIR_BUFFER`,
      `RTXDI_RIS_BUFFER`, `RTXDI_NEIGHBOR_OFFSETS_BUFFER`). Each is defined as a `T*` field of
      the pass's frame block, so buffers stay by device address.
    - **Forge's own Rust for the C++ contexts:**
      - the reservoirs' pitch, in blocks of 16 pixels;
      - the parameter blocks of each pass;
      - the lights' power texture, with its mips;
      - the spatial neighbours' offsets;
      - the map of each lamp's index from one frame to the next, since the kept lamps change
        as the camera moves.
    - **Passes:**
      - presample the lamps by power into tiles of 1 024;
      - initial samples, 8 a pixel;
      - one spatiotemporal resampling;
      - shading, with one shadow ray.
    - **The comparison:** the same frames under `--lamps restir` and under the light lists, as
      images, shimmer (DLAA, a still camera and a turn) and cost. ReSTIR can take all 325
      lamps lit at 22:00, not only the 64 nearest that the far rooms' glow stands in for.
    - **Compiling:** `slangc` gets `rtxdi-sdk/Include` on its include path when the directory is
      there. Without it (CI, other machines) the switch is absent.
    - **Checked before any code:**
      - The setup: Forge's slangc flags, a stub bridge and the buffers as frame-block
        pointers.
      - It compiled `RTXDI_SampleLocalLights` (power RIS),
        `RTXDI_DISpatioTemporalResampling` and `RTXDI_StoreDIReservoir` to 72 KB of SPIR-V that
        `spirv-val` passes for Vulkan 1.3.
      - The bridge includes `Rtxdi/Utils/RandomSamplerState.hlsli` first.
      - Slang warns 11 times about a uint tested as a bool inside the library. That file is
        compiled with `-warnings-disable 30081`.
      - The parameter structs live in the library's `.h` files. Forge's Rust mirrors them
        (`forge_render::restir`, the frame block checked at 288 bytes).
  - **Built the same day (`--lamp-restir`, `forge_render::LampRestir`, `shaders/restir.slang`).**
    - **One pass before the resolve, "shadow/lamp restir":**
      - the pixel's surface, stored for the next frame's reuse;
      - its initial reservoir: up to 8 of its place's lamps (the light lists) picked uniformly,
        or all of them when there are no more, each at a point of its sphere sampled by solid
        angle; one kept by its light, and its shadow ray;
      - RTXDI's spatiotemporal resampling against the last frame's reservoirs, through TAA's
        reprojection, with `Basic` bias correction;
      - the kept sample's shadow ray, and its light times the reservoir's weight. The resolve
        adds it, with its highlight towards the lamp (`lamps_at`).
    - **The bridge:**
      - surfaces in the scene's frame, an octahedral normal and the place;
      - lamp indices mapped from one frame to the next by each lamp's middle;
      - a lamp gives no light where it doesn't light the place, with the storey band as the
        lists have it.
    - **The comparison, the village at 22:00 under DLAA, against the light lists and their
      penumbra filter:**
      - **Images** (mean ꟻLIP): hall 0.038, room 0.076, landing 0.014, door 0.019, house
        front 0.006, main street 0.000. ReSTIR's room is brighter on two walls; why is still
        open.
      - **Shimmer**, a still camera's frames 10 apart: indoors 0.20–0.51 % of pixels change
        against 0.000–0.005 % (TAA: 1.6–2.5 % against 0.14–0.17 %). A house front at night
        stays even: 0.04–0.10 % both ways.
      - **Cost**, the serial frame, best of three: hall 5.51 against 5.96 ms. The pass takes
        0.50 ms, against 0.81 for the rays and the filter, and the resolve no longer sums the
        room's lamps.
    - **Every lamp** (`--all-lamps`: all 325 lamps on at 22:00, and the 49 street lamps'
      98 heads):
      - ReSTIR costs +0.12 ms indoors and +0.18 on a house front; the light lists +0.54 on the
        house front.
      - At one fixed exposure the street changes by 0.0004. It first looked darker because
        more lit windows lowered the automatic exposure.
  - **The street lamps lit (the owner, 2026-10-18).** The 49 street lamps had been glowing
    heads only. The pools on the ground came from the probes, which saw the lenses' glow and
    spread it in blocks of their grid, away from the lamps. The moon's shadow of each pole
    crossed its "pool".
    - **Now a lamp under each head** (`forge_render::lamps::street_lights` since #296, under
      `forge_geom::street::lamp`'s lenses):
      - its intensity is the lens's luminance times its area: 109 cd, so 1.9 lux under an 8 m
        lamp, at 2 700 K;
      - on from 17:00 to 7:00;
      - it lights the outside's region only, not the rooms behind the windows
        (`Lamp::outside`);
      - the owner tried 7.5 and 12 lux, LED street lighting's range, and kept 1.9.
    - **Its head is full cut-off** (`lamp_beam`): all its light down to 60° off the vertical,
      none from 85°. A bulb lit every way had left a dark funnel on the facade over each lamp,
      from its own housing's shadow. Since #288 that cut-off is the lamp's own emission
      (`Emission::STREET`), not its region's.
  - **Each lamp's emission** (#288, 2026-10-18; asked by a scene built outside the repository
    with downlights, porch lamps, floods and outdoor globes). Before, a street lamp's cut-off
    followed from its region: an outdoor globe got the street's beam, and nothing else could
    have one.
    - **`Lamp::emission`:** `Emission::Every` (today's lamps) or `Emission::Cone { axis,
      inner_cos, outer_cos }`, a smooth step from all of the light within the inner angle to none
      beyond the outer. `candela` is the intensity along the axis. `Emission::flush(axis)` is a
      dome or a diffuser on a ceiling or a wall: none behind its mount, a quarter of an omni
      lamp's lumens at the same peak.
    - **One function, every path:** `lamp_beam` reads it, and the resolve, the light lists'
      picks, the probes' and the mirror rays' hits and ReSTIR DI's target all call it.
    - **On the GPU:** `GpuLamp` grew from 64 to 96 bytes (the axis with the inner cosine, then
      the outer cosine) and keeps its eight regions.
    - **Checked:**
      - the street's cut-off, now the cone down at 0.5 and 0.087, is the same arithmetic as
        before. The village at 21:00 (the main street and the junction under the light lists,
        the street under ReSTIR, the hall's house lamps) differs in 0 px, max channel error 0;
      - unit tests: the cone's share against the old beam's expression to the bit, the dome's
        half at 60° and its lumens (π against 4π);
      - the lab's lamp room with a downlight and an uplighter (a test, not kept): the floor's
        pool and the walls' arch alike under the light lists and ReSTIR.
    - **Left for later:** a lampshade (dim sideways, open up and down), and a fitting radius
      inside which the probes' hits ignore the lamp (#294).
  - **Shares and fades in the light lists' choice** (#295, 2026-10-18; seen in a town of a few
    hundred house lamps built outside the repository). The 64 nearest lamps were kept whatever
    their kind, so the nearest rooms crowded the street lamps out, and a lamp went on or off at
    once at the cut.
    - **Shares:** the street lamps (`Lamp::outside`) keep up to 24 of the 64
      (`STREET_SHARE`), the others the rest. The slots one kind leaves go to the other's nearest.
    - **Fades:** a kept lamp's light fades out over the last fifth of the distance to its cut, at
      least 2 m. The cut is the distance of its kind's nearest lamp not kept, or where its light
      stops counting (its reach and the 50 m margin). The cut moves with the camera without
      jumps, and two lamps that change places at it are both faded out.
    - **Checked:**
      - a unit test walks a camera 40 m in 1 cm steps across 200 lamps. Lamps enter and leave the
        kept 64 dozens of times, and no lamp's light changes by 1 % of it in a step. Without the
        fade, the same test sees jumps of all of it;
      - another test keeps 24 street lamps 60–89 m away against 100 house lamps within 10 m;
      - the village at 21:00: the hall unchanged (0 px), the junction 102 px, and the main
        street 974 px, where a lamp 80 m away is fading out towards the end of its reach.
  - **Lamp shadow rays start where the traced cut allows** (#293, 2026-10-18; seen in the same
    town). They started 15 cm off the surface (`SHADOW_BIAS`, the sun's), so nothing within
    15 cm of a lit surface shadowed it. A fitting's housing 7 cm deep let its bulb light the
    ceiling round it in a halo, and a sofa against a lit wall cast nothing on it.
    - **The start** (`lamp_shadow_start`) is twice the gap the traced cut may leave in front of
      the drawn surface: the cut's error, and the drawn cluster's own where a coarser level is
      drawn, scaled with the instance. It is at least 2 cm and at most the old 15 cm, and a
      terrain keeps its sun rays' start. Meshes under the cut's budget are cut at full detail, so
      near the camera they start at 2 cm.
    - **Where:** the resolve's lamp rays and the penumbra filter's (`lamp_rays_main`). The
      probes' and mirror rays' hits keep 15 cm. There the start changed the rooms' indirect
      light by up to 1 EV, and not steadily: 8 cm halved a hall's, and 2 cm added half a stop
      (#298, below).
    - **The sun's rays, by the mesh** (#326, 2026-10-19). The case: a 4 cm door leaf shut in
      a 20 cm wall let the sun light the lining and the floor behind it.
      - `MeshletSceneBuilder::set_shadow_start_by_cut` gives chosen meshes the lamps' start for
        the sun's rays too (a negative `shadow_start`, read in `shadow_bias`).
      - Every other mesh keeps 15 cm, so nothing drawn changes: the map check's 13 images and
        the lab's 84 captures are at 0 pixels, the #71 flake aside.
      - **Every mesh's start since 2026-10-21**, but a terrain's (its own, #96) and a mesh with
        cut-out sections' (trees, gratings: 15 cm). The leaf cards' normals are bent out from
        the crown to shade it as a volume. From 2 cm they shadowed each other inside it, and
        the crowns lost their sunlit leaves (the junction view's ꟻLIP mean 0.066 with them,
        0.024 without).
      - What it shows: the cornice's tiers shade the faces under them, a door's leaf its
        lining, a capsule and the lab's spheres the ground they touch, the material slabs'
        edges the floor. No acne on the coarse cuts: the start grows with the cut's error up to
        15 cm. The map check moved by up to ꟻLIP mean 0.045 (the vault's view), the sentinels by
        up to 0.0045 (the lab's materials).
  - **A hit's lamp ray is aimed from where it starts** (#298, 2026-10-19).
    - **The cause:** `hit_lamps` started its ray 15 cm off the hit along the normal but aimed it
      along the line from the hit itself. The ray ran beside that line and ended up to 15 cm
      from the lamp's middle, in its globe or its fitting, or past it. How often it ended inside
      the fitting depended on the start, hence the swings. The resolve's rays (`lamp_ray`) were
      always aimed from their start.
    - **Now:** `hit_lamps` traces `lamp_ray` too.
    - **Checked** (21:00, DLAA, a fixed EV, the probes' light alone):

      | Start | The hall, before | The room, before | The hall, after | The room, after |
      |---|---|---|---|---|
      | 8 cm, against 15 | −1.00 EV | −0.55 EV | −0.003 EV | −0.005 EV |
      | 2 cm, against 15 | +0.54 EV | +0.30 EV | −0.009 EV | −0.009 EV |

      At 15 cm the hall is 0.55 EV and the room 0.31 EV brighter than before, the light the
      fittings had taken. The start stays 15 cm, since it no longer matters.
    - **Checked** (old start against new, 21:00, DLAA):
      - the village's room and window recess: 5 500 and 4 400 px gain shadow (the window frame
        against its reveal, the table's legs under its top), 0.0007 EV or less;
      - the hall: 0.0016 EV darker;
      - the lab's lamp room: the floor before the board is lit again under the board, which
        floats 15 cm over it. A ray from 15 cm up started at its lower edge and missed the gap.
  - **A probe's hit takes a lamp's light from no nearer than 25 cm** (#294, 2026-10-18).
    - **Before:** the surfaces inside a fitting and right by its bulb (a housing's inside, the
      ceiling over a globe) were lit at tens of thousands of lux in the probes' rays. The probes
      spread that over the ceiling round the fitting in a halo; a scene built outside the
      repository lined its fittings with black to hide it.
    - **Now:** `lamp_weight` takes the distance from no nearer than the bulb's radius or a given
      least distance. `hit_lamp_weight` gives a probe's hit `PROBE_LAMP_NEAR`, 25 cm, both for
      its pick and its light. The resolve lights those surfaces directly, each pixel its own, as
      before. A mirror ray's hit keeps the bulb's radius: a mirror shows the fitting as it is lit.
    - **Checked** (21:00, a fixed EV, the probes' light alone): the village's hall −0.05 EV and
      room −0.03 EV, from the hot spots by the bulbs. The light lists' picks are unchanged to the
      bit (they pass no least distance).
    - **The probes leave the street lamps out** (`hit_lamp_weight`, and the lenses tagged
      `RenderLayer::lamp`). Their bounce came back in sharp blocks of the probe grid on the
      grass (#254).
    - **The kit's window frames are closed on every side.** Through the clear glass a room saw
      the frames' missing backs, which read as gaps between the frame and the walls.
  - **Option 2 (the owner's choice, 2026-10-18), behind `--street-restir`.** Each frame keeps
    only the 64 nearest lamps within 50 m, so from 100 m away a street went dark. Three ways
    were put to the owner: Olsson's clusters (AMD's Forward+ patents to check first), ReSTIR
    for the street lamps alone, or every street lamp in the light lists (too costly). The
    owner took the second.
    - **How it works:**
      - every street lamp is kept, however far (`LampChoice::Streets`);
      - ReSTIR takes them alone (its candidates from the outside's light list); the light lists
        and their filter take the houses' lamps;
      - the resolve adds both (`LampShadows`, two sampled indices in `lamp_share`'s halves).
    - **Measured:**
      - far streets keep their light;
      - but on a lit street a still camera's frames differ on 0.9–1.6 % of pixels, against
        0.002–0.005 %: one sample a pixel, with no denoiser;
      - +1.3 ms on a street view (8.55 against 7.27 ms).
    - **So it waits behind its switch until ReSTIR's street light has a denoiser** (NRD's
      ReLAX or REBLUR, or the lamps' own filter on its ratio; patents to check). The default
      keeps the street lamps in the light lists, the 64 nearest.
  - **The street lamps beyond the set, unshadowed** (#284, 2026-10-19; the issue's first
    direction, the owner's "follow recommendations"). The 64 nearest keep their shadow rays and
    the penumbra filter; every other street lamp lights the outside without a shadow ray.
    - **How it works:** `lamps::FarLamps` lists the street lamps in a grid of 8 m columns, each
      lamp in those its reach overlaps (`MeshletSceneBuilder::set_far_lamps`, the decals' hash of
      columns). Each frame, `set_lamps` gives every one of them the share of its light the set
      does not light (`choose_shares`): 0 for a lamp kept whole, 1 for one left out, and in
      between for a lamp fading at the set's cut, so a lamp fades from one path into the other.
      The resolve adds those shares after the penumbra filter (`far_lamps_at`), outside only.
      `--no-far-lamps` is the A/B.
    - **Measured** (#284's path: the village at 22:00 under DLAA, EV 3, receding from the junction
      12 m up, 100 m at 2 m/s): against `--all-lamps` (every lamp with its shadow rays) a frame
      differs on 1 200–4 300 px along the path, against 2 300–26 200 px before; what is left is
      the far pools' shadows. `shading/standard` 1.026 → 1.072 ms and the frame 4.92 → 4.96 ms
      over the path; `--all-lamps` takes 6.33 ms. Capture:
      `reports/2026-10-19/village-far-street-lamps.png` (80 m out, brightened four times; before,
      with, and every lamp shadowed).
    - **The windows' far glow from their rooms' own lamps** (#284's second direction). At 80 m
      what still differed from `--all-lamps` was the windows: a pane's glow followed its
      facade's per-window draw and its room's lamps the room's own, so far panes glowed white or
      television-blue over dark rooms and stayed dark over lit ones. Now each clear pane takes
      its room (the probes' regions, 30 cm either side) and that room's strongest lamp that is on
      in its storey band (city-plan's `room_lamps`, `GlassPanes::lamp`): it glows with 0.02 cd/m²
      a candela of that lamp's light, in its colour, times the share of it the frame leaves unlit
      (`lamps::unlit`, `Glass::set_unlit`), at any distance; a room whose lamps are off stays
      dark. Seen aslant, the glow shrinks with the share of the pane the room shows through past
      a reveal half as deep as the opening is wide (`REVEAL_DEPTH`). Along the same path, the
      light the windows differ by from `--all-lamps` at 60–100 m went 240–301 → 18–45 (the sum of
      the differing pixels' linear light); the frame's cost is the same (`glass/draw` 0.006 ms).
      Capture: `reports/2026-10-19/village-far-windows.png` (80 m out, ×3; the old glow, the
      rooms' own, every lamp lit).

**Amended: every lamp by ReSTIR DI, denoised** (the owner, 2026-10-31, #372: "ok, go with ReSTIR
DI and the denoiser"). The Bistro at night has 132 lamps, each reaching 27 to 70 m, so the 64
nearest left its far street and arch black, and clusters would not shorten a pixel's list. Measured
on its night view (the probes, the 5070 Ti, mean of 298 frames): the 64 nearest by the light lists
15.1 ms a frame, their passes 5.7 ms; every lamp by the lists 22.6 ms (10.2); every lamp by ReSTIR
15.1 ms (2.7), but grainy without a denoiser and 0.24 stops dark (it took every lamp as a sphere,
the flat lamps of #369 and #370 never reached it). The order:
1. ReSTIR's bridge takes the flat lamps (their rectangles, sampled by area), and its light is
   checked against the light lists' with every lamp: the same direct light, one estimated.
2. Its light denoised by NRD's REBLUR (already in the frame for the bounce, D-088), designed before
   it is built.
3. The lanterns' profile (#372's second fault).
4. The default where RTXDI's headers are present, once Forge's path with the lamps on matches it
   off within about 0.1 in the Bistro and the rooms, a still camera's shimmer is near the light
   lists', and the cost holds; the light lists and their filter stay where they are absent (the
   SDK policy: GitHub builds, CI).

**Step 1 built (2026-10-31): ReSTIR's light matches the light lists'.** Measured as each tile's
light over a reference's, the plain mean of the 144 tiles' ratios (the mean of their logarithms
reads one sample's noise as darkness); the judge, Forge's path tracer with every lamp on (the same
lamp model, integrated exactly). Five faults, each found by taking one part away:
- **The flat lamps** (`restir_flat_sample`): a point of the rectangle uniform in area, its density
  over solid angle the distance squared over the area and the lamp's cosine, its luminance the
  intensity over the area.
- **Permutation sampling off** (`RestirSettings`'s reuse): RTXDI's permutation hands a pixel the
  last frame's reservoir of a pixel up to 3 px away; the Bistro's night stood 0.2 stops under the
  lists with it, whichever bias correction, and level with the resampling without reuse without it.
- **A sphere's shadow ray ends where the sphere is nearest**, as the lists' (`short_of`): run on to
  a point by the sphere's edge it went into the lanterns' frames and glass, 8 % of the light.
- **Pairwise MIS** for the bias correction (RTXDI's `PAIRWISE`, no rays): under `Basic` and
  `RayTraced` the Cornell box's walls under its one lamp stood 10–15 % under the lists, with the
  tall block's shadow drawn in patches; pairwise matches (1.000).
- **RTXDI's reservoirs read as D3D12 reads a buffer** (`RestirReservoirs`): its pairwise reuse loads
  the reservoir of a pixel it found no surface at, which can lie off the screen; by device address
  Vulkan lost the device in every scene whose surfaces reach the screen's edges. Past the arrays a
  read gives an empty reservoir and a write is dropped.
- And `RestirFrame::candidates` points a pixel always (8), spread over its place's lamps when it has
  fewer: one point of the Cornell box's one lamp a frame was too few.

| Scene (DLAA) | the lists | ReSTIR before | ReSTIR now |
|---|---|---|---|
| Bistro at night, every lamp, against Forge's path | 0.987 | 0.775 | 0.969 (median 0.977) |
| Cornell box, against the lists | 1 | 0.904 | 1.000 |
| Kitchen (clay), against the lists | 1 | 0.941 | 0.943 (0.949 without reuse) |
| Bedroom (clay), against the lists | 1 | 0.939 | 0.940 |

The kitchen's and the bedroom's 5 % are between the two estimators, not in the reuse; the likely
part is the normal: the lists light the diffuse with the mapped normal, ReSTIR with the
interpolated one (its light reaches the resolve as illuminance), to settle with step 2, which
changes what ReSTIR hands the resolve. The Bistro at night costs the same with every lamp by
ReSTIR as with the 64 nearest by the lists: 15.1 ms a frame, "shadow/lamp restir" 1.9 ms. The
light lists are untouched: ReSTIR stays behind `--lamp-restir` until step 4.

**Step 2's design: the lamps' light denoised** (2026-10-31, before code).
- **Which of NRD's denoisers.** NRD says RELAX was made for RTXDI's signal; REBLUR is the one Forge
  already runs (the bounce, D-088), its settings mirrored and checked against MSVC's layout. First
  REBLUR (a second instance of `ReblurDiffuse`, its own history), RELAX only if REBLUR lags or
  smears the lamps' shadows.
- **Not mixed into the bounce's REBLUR**, though NRD allows direct and indirect light together: the
  bounce is in the resolve's units, light over the sun's (at night the moon's 0.17 lux), where a
  lamp's thousands of lux overflow the half floats (#368); and the lamps need it where the bounce
  is off. The lamps' instance takes lux over 256 (a bulb's 65,000 lux to 254).
- **What it denoises: the diffuse light first** (2a). The ReSTIR pass writes a second image: the
  illuminance in YCoCg and the distance to the sample's point, normalised as the bounce's
  (`reblur_hit_distance`, `HIT_DISTANCE`), as NRD asks for direct light. REBLUR reads the guides
  the sun's shadow rays already write (the interpolated normal, the view depth) and TAA's motion
  vectors. The resolve takes the denoised illuminance for the lamps' diffuse light, and keeps the
  sample's own for the highlight.
- **The highlight next** (2b): REBLUR's diffuse-and-specular denoiser on the sample's GGX light
  over the surface's directional albedo, so a glossy surface's highlights of many lamps settle as
  their diffuse light does.
- **Without NRD** (no library, a GitHub build): the resolve reads the sample's light as now.
- **Checks:** the tile ratios against Forge's path with every lamp (the Bistro at night) and against
  the lists (the rooms); a still camera's frames 10 apart (the lists' 4,230 pixels in the Bistro at
  night); the camera turning (`--pan`), for REBLUR's lag; the cost.
- **Patents:** none new; NRD's REBLUR as provided, as the bounce's and SIGMA (D-049).

**Step 2a built (2026-10-31): the diffuse light denoised.** `LampRestir::draw` returns the sample's
light and the same for REBLUR (`RestirLight`); city-blocks denoises the second by its own
`DiffuseDenoiser` on the sun's shadow rays' guides; the resolve, told by the frame's
`lamp_restir_every` that ReSTIR lights every lamp, takes the denoised light for the lamps' diffuse
light (the low half of the lamp code) and the sample's for their highlight.
- **The Bistro at night, still:** 0.955 of Forge's path with every lamp (median 0.962; 0.969
  without the denoiser), the frame 15.9 ms (REBLUR's second instance about 0.6 ms). The walls and
  the street are smooth; what still sparkles is the sample's highlight (gone with it left out):
  step 2b.
- **The rooms:** the denoiser keeps their light (the Cornell box 0.995 of the lists, the kitchen
  and the bedroom as before it).
- **Turning** (`--yaw-rate 6`, frame 280): 0.911 of the lists at first. RTXDI's disocclusion boost
  (8 of the last frame's neighbours where it has no history of the pixel) gave the new strip at
  the screen's edge the old edge's reservoirs, clamped there: with one, 0.938; without any reuse,
  0.962. The rest is open, to settle before step 4.
- **The review's fixes:** where the pairwise reuse streams no neighbour, RTXDI's spatiotemporal
  pass leaves the pixel's own sample a weight of 0 (its spatial pass sets 1), the pixel black and
  the frames after it dark: the pixel's own reservoir is kept then. A reservoir of no sample (a
  place without lamps, a room's pixel under `--street-restir`) no longer counts as a neighbour
  (its place marked empty).
- **Tried, no gain:** REBLUR's firefly suppressor off for the lamps (0.913 against 0.911 turning,
  0.955 against 0.954 still).

**Step 2b's design: the highlight denoised** (2026-10-31, before code).
- **The obstacle:** the highlight needs the pixel's material (its mapped normal, lobe and
  reflectance, D-086), which only the resolve works out, inline, from the visibility buffer (its
  texture coordinates and their derivatives, the tangent frame, skinning, the planets' tiles). The
  ReSTIR pass has the geometry alone.
- **The way:**
  1. The resolve's surface and material inputs become one function, called by the resolve as
     before (the asteroid sentinels and the captures unchanged to the pixel, #355) and by the
     ReSTIR pass.
  2. The ReSTIR pass writes the sample's diffuse light with the mapped normal (the lists' normal:
     likely the rooms' 5 %), its highlight over the surface's directional albedo (the split sum's
     term the resolve multiplies by, as NRD asks: demodulated), and the guide REBLUR's specular
     needs (the mapped normal and the linear roughness).
  3. REBLUR's diffuse-and-specular denoiser (NRD's `REBLUR_DIFFUSE_SPECULAR`, the same settings,
     its own profiler group `shadow/lamp REBLUR`) denoises both; the wrapper takes two outputs.
  4. The resolve reads them through the lamp code's two halves (the sample's light no longer
     needed where both are denoised), and multiplies the highlight back by its own albedo: no new
     push fields (the resolve's are at Vulkan's 128 bytes).
- **Not this way:** writing the highlight from the resolve and denoising it after (three more
  images from every resolve class, the push full); the row's factors in the ReSTIR pass (the
  scooter matte again, D-086); the sample's own highlight clamped (it sparkles under DLAA).
- **Checks:** the sentinels at 0 px after the refactor; then step 2a's.

**Step 2b built (2026-10-31): the highlight denoised as the diffuse light.**
- **The refactor:** the resolve's surface and material row (`shade_pixel`'s first block) are
  `pixel_shading`, moved verbatim. The asteroids (#355: `ast240`, its A/B twin, `ast-notaa600`),
  `lab-materials60`, `lab-lamps60`, `city60`, `gallery60` and `planet-ground` on both paths:
  0 px from the accepted set, and the A/B harness and mesh against fallback 0 px. The AMD
  iGPU's check (#355's oracle, `tools/amd-check.sh`, the same day): meshlets, asteroids and the
  lab's yard, smoke yard and lamp room each identical from run to run and the mesh path to the
  fallback, no validation messages (the asteroids' one line is #367's notice).
- **ReSTIR's pass** calls it and `standard_inputs` (`row_inputs` for the other classes), and
  writes the sample's diffuse light by the mapped normal, its highlight as `lamp_add`'s (GGX,
  the lobe widened by the lamp's sphere) over the surface's directional albedo
  (`environment_specular`), and REBLUR's guide (the mapped normal, the linear roughness).
- **NRD's `REBLUR_DIFFUSE_SPECULAR`** denoises both (`LampDenoiser`, the wrapper's second output,
  its passes `shadow/lamp REBLUR ...`); the resolve reads them through the lamp code's two halves
  and multiplies the highlight back by its albedo.
- **The Bistro at night:** the sparkle is gone; 0.930 of Forge's path with every lamp (median
  0.932; the default's 64 nearest 0.797), 17.5 ms (`docs/PROFILE.md`).
- **The rooms:** unchanged by it (the Cornell box 0.994 of the lists, the kitchen 0.936, the
  bedroom 0.935): their 5 % is not the normal either.
- **Turning** (`--yaw-rate 6`): 0.908 of the lists (their reference taken before the lanterns'
  profile, which takes about 1 %): still the open point before the default.

**Where step 4 stands (2026-10-31): not the default yet; the shimmer.** The Bistro's night view,
a still camera's frames 280 and 290 (pixels differing by more than 2 levels), the light as the
tiles' mean ratio to Forge's path with every lamp, the frame on the 5070 Ti:

| | shimmer | light | frame |
|---|---|---|---|
| the light lists, the 64 nearest (the default) | 4,892 px | 0.797 | 15.3 ms |
| the light lists, every lamp | 6,884 px | 0.987 | 21.9 ms |
| ReSTIR, both lights denoised (REBLUR 30 frames, 8 candidates) | 94,909 px | 0.929 | 16.6 ms |
| the same, REBLUR 63 frames, 16 candidates | 42,929 px | 0.915 | 16.9 ms |
| the same, three neighbours with visibility (taken, below) | 24,660 px | 0.905 | 19.0 ms |

- ReSTIR lights the whole street for 5 ms less than the lists with every lamp, but shimmers six
  times as much: REBLUR's light still moves at the pixel on the cobbles, the basket's leaves, the
  wires. The owner sees shimmer at once: it stays behind `--lamp-restir`.
- **Spatial reuse** decorrelates it (three neighbours: 26,752 px) but loses 40 % of the light
  (0.592): pairwise MIS weighs a neighbour's lamp by its light without its visibility, and in
  the Bistro's street pixels a few apart see different lamps round the walls. The same reuse at a
  disocclusion is what darkened the turning camera. RTXDI's answer is `RayTraced` bias correction
  (visibility in the weights), which in Forge's bridge darkened the Cornell box's lit walls too,
  for a reason not found yet (step 1).
- **Found the same day: RTXDI's naive discount.** Forge had `discountNaiveSamples` on: RTXDI
  skips a neighbour of two samples or fewer, and a reservoir starts with one, so a pixel's
  history never grew; under `Basic` the skipped neighbour still counted in the normalisation, half
  the light (the Cornell box 0.892 of the lists, 0.995 without the discount). With it off, the
  reuse takes visibility in its weights (`RayTraced`), keeps an occluded sample in its reservoir
  (emptied, its count gave the neighbours nothing: 0.858 → 0.905) and three neighbours (and three
  where it has no history): the shimmer 42,929 → 24,660 pixels for 2 ms. Turning at 6° a second,
  0.906 of the lists with every lamp (both with the lanterns' profile; the 9 % of step 2a was
  against a reference taken with the cone): about 1 % lost to the motion. The rooms lose 1 %
  (the Cornell box 0.990, the kitchen 0.925, the bedroom 0.924 of the lists).
- **Still four times the lists' shimmer.** The highlight is about 10,000 of the 24,660 pixels,
  the diffuse light the rest; REBLUR's fast history (6 → 16 frames) changed nothing (24,441).
- **The split, built the same day** (`--lamp-restir --lamp-restir-near N --all-lamps`): the light
  lists light the N nearest lamps with their filter, ReSTIR the rest (the frame's
  `lamp_restir_from`; the lists skip ReSTIR's lamps through `lamp_by_restir`, ReSTIR's
  candidates are drawn among the far ones), its two denoised lights packed into one image
  (`LampRestir::pack`, "shadow/lamp restir pack" 0.02 ms) beside the lists' share:

  | | shimmer | light | frame |
  |---|---|---|---|
  | the lists the 64 nearest, ReSTIR the rest | 7,788 px | 0.971 | 26.5 ms |
  | the lists the 32 nearest, ReSTIR the rest | 9,519 px | 0.944 | 23.8 ms |

  Its image is the lists' nearly (the far light's noise dim), but it costs more than the lists
  with every lamp: ReSTIR's pass is 4.7–5.5 ms with its visibility rays, beside the lists' rays,
  filter and resolve. The lab's lamp room and smoke yard, the materials and the city: 0 px.
  Open (its review): a lamp crossing the N-th place is taken or dropped by the lists at once
  while REBLUR's far light keeps or lacks it for its history, counted twice or missing for a
  few frames as the camera moves; a band where both share it (as `faded` at the 64 nearest's
  cut) would hand it over. Fixed: a lamp nearer last frame no longer counted as drawable by the
  last frame's neighbours.
- **For the owner (#372 reopened):** at the Bistro's 132 lamps the light lists with every lamp
  give the best image for its time (0.987 of the path, 6,884 px, 21.9 ms: +6.6 ms over the 64
  nearest); ReSTIR is 3 ms cheaper and shimmers 3.6 times as much; the split is for scenes of
  many more lamps, where the lists' cost grows with each. **Recommended now:** the lists with
  every lamp the frame's lamps reach (the 64-lamp cap raised, clustered lists to bound a pixel's
  lamps later), ReSTIR kept behind its switch while its shimmer is open. Then the lamp model's
  13 % over the traced glow and the rooms' 5 %.
- **Decided** (the owner, 2026-10-31: "Follow recommendations"): the light lists light every
  lamp whose light reaches within 50 m of the camera, up to the buffers' 512 (`MAX_LAMPS` =
  `LAMP_CAPACITY`, the 64 before); ReSTIR stays behind `--lamp-restir` (and the split behind
  `--lamp-restir-near`) while its shimmer is open. The Bistro at night by default: 0.987 of
  Forge's path with every lamp, 6,840 px of shimmer, 22.1 ms (the 64 nearest: 0.797, 4,892 px,
  15.2 ms); its far street and arch lit. The village at 22:00, where each room's lamps are its
  own list: the street 7.79 → 7.80 ms, the room 7.85 → 7.90, the landing 7.38 → 7.40. Clustered
  lists (D-065's part 1) come when a pixel's place holds more lamps than it can afford.

**Step 3 built (2026-10-31): a lantern's light by its glass** (`Emission::Prism { up, down }`):
at the elevation φ, over its side's intensity, cos φ for its upright panes and its glass's
measured shares seen from above and below by the sine (the Bistro's lanterns 0 and 0.32), where
the cone before it sent the side's whole intensity straight down. Every path reads it through
`lamp_beam`. **The fault was smaller than #372 supposed:** Forge's path with every lamp over
itself with the glow traced went 1.136 → 1.127 (the plain mean of the tiles' ratios; signed
mean |log2| +0.173 → +0.161). The rest of the 13 %, brightest on the far left street's walls round
its lanterns, is still to find: the fittings' frames and glass block the traced glow but not the
lamps' rays, which end at their spheres; or a piece's intensity as its glass's area. The lab's
other lamps are unchanged (`lab-lamps60`, `lab-smoke360`: 0 px on both paths).

**`--moonless`** (2026-10-31, the owner asked whether the Moon lit the Bistro's night): the night
without the Moon's light (its key light, its scattering, its disc, its fill), its units kept.
In the Bistro's night view the Moon is 0.4 % of the light (the tiles' mean ratio 0.996, the
median 0.999): not what darkened the far street. `tools/gi-compare.sh`'s nights run moonless from
now, as Falcor's references are lit.

**A lamp's shadow optional** (2026-11-01, #386, under the owner's "Follow recommendations until 10
am"): `Lamp::shadows`, true for every lamp but an authored scene's whose data marks it without (a
shipped city's raster mode shadows 518 of its 6,426 static lights, its ray-traced modes all of
them). A shadowless lamp's light takes no shadow ray: not in the light lists (`lamp_add`,
`lamp_look`), at the rays' hits (`hit_lamp_through`: the bounce, the probes, the reflections) nor
in ReSTIR DI (its sample seen without one, `RAB_GetConservativeVisibility`). The record grew 96 →
112 bytes (`flags`): the Bistro's night 18.76 → 18.76 ms with the lists, 15.66 → 15.42 with ReSTIR
(two runs each); with every lamp shadowed the images are unchanged (the lab's set and the Bistro's night with its
lists, ReSTIR DI and the street lamps' ReSTIR: 0 px against the build before; a first run of the
base's lists differed from its own second in 234 pixels, a flake of the base's). The private
district's night under DLSS Quality at 2560 × 1440 with only the lights its source's raster mode
shadows (465 of 6,640 within 256 m): 14.6 → 14.0 ms (two runs each). Its shadow rays are not where
its night goes (the lamps' weighing is); the option stays for authored data, and whether the
district matches the raster mode (its lights then reach through walls) or the ray-traced one is the
owner's call.

**Flat lamps weighed cheaply at the rays' hits** (2026-11-05, #396): a ray's hit (the probes', the
bounce's, the caches') picks one lamp of those in reach by its weight, its unshadowed light. A flat
piece's weight was its rectangle's exact light (`lamp_rect_share`, a clipped polygon's four edges)
for every lamp weighed. The private district, with its long tubes as flat pieces (its viewer's), paid
about 0.3 ms for it whatever their number (145, 366 or 1,155 of them).
- **Now:** `lamp_flat_hit_weight`, the light from its rectangle's point nearest the hit (Lambert's
  cosines from there, each at least 0.05, over the distance squared), none over the exact light's
  bound (π over its area). The picked piece's light then takes its exact weight over this one
  (`hit_lamp_exact_factor`): unbiased, since the cheap weight is over 0 wherever the exact one is.
- **A mirror's hit** (its light neither accumulated nor denoised) still weighs its flat pieces
  exactly, their light without noise.
- **The numbers:**
  - **The district's path** (2560 × 1440, DLSS Quality, interleaved after a warm-up), its tubes
    of 4 m or more as flat pieces against as points:
    - weighed exactly: 16.13 and 16.18 ms against 15.83 and 15.86 (+0.3);
    - from their middles: about even, within the runs' noise;
    - from their nearest points (kept): 15.69 to 15.88 against 15.46 to 15.72 over three pairs
      (+0.19).
  - **The Bistro's night** (its 160 glowing pieces' lamps, the probes' light): the 16 × 9 tiles'
    mean |log2| 0.000 against the build before, the frame ×1.000. Pixels differ by noise alone
    (11 % of them, 1,011 by over 5 %).
  - **The village's map-check views:** 0 to 36 pixels from the build before (the bounce's noise).
- **Then** (#396): a tube emission for the strips themselves (below).

**A tube emission** (2026-11-05, #396): a tube or strip of light as one lamp, `Emission::Tube`: a
segment along an axis, as thick as the lamp's radius, glowing as a Lambertian cylinder. Its
intensity is its candela across its axis times the sine from it; its flux is π² times that candela.
- **Why:** the private district's strips, one lamp each in place of one point or two flat pieces
  back to back (above).
- **What it did not cure** (corrected the same evening; the entry first said a sign's strip washed
  its letters from 30 cm, a figure not measured). The sign's hot spot that started this came from
  a strip 4 cm in front of its letters, the source's stand-in for the sign's glow on the street.
  Point, flat pieces and tube each lit the letters more faithfully, and burnt a band across them.
  The cure is in the district's viewer: such a light stays, and the glow it sits on takes no
  light. The source's own frames show those letters evenly lit by their glow.
- **Its light:**
  - *Diffuse:* its line's, in closed form (Nishita, Okamura and Nakamae 1985; Picott 1992), the
    segment clipped at the surface's horizon (`lamp_tube_share`). The CPU's `lamps::tube_share`
    is tested against a sum along the tube.
  - *Highlight:* from its point nearest the mirror ray, lit by the line's light (its sine over the
    distance squared, along it) and the lobe's share of it: Karis 2013's representative point
    and line normalization.
  - *Shadow rays:* to points along its part above the surface's horizon (`lamp_tube_target`);
    the penumbra filter's spread evenly along that part.
  - *ReSTIR DI:* a point uniform along it, the tube a ribbon as wide as it facing the surface. Its
    estimate averages to the closed form.
  - *The fog:* its line's light without the surface's cosine; the phase and the shadow ray from
    its nearest point.
  - *Reach:* its range runs from its line: its light lets go between 80 % and all of it from the
    segment's nearest point (`lamp_tube_window`). The lists take it by a sphere of that plus its
    half length (`Emission::extent`).
    - **First built otherwise** (1cc2f749): a range as a sphere about its middle, chosen because it
      cost less over the flat pieces and a sign looked alike. The private district's bar strips, 7 and
      8 m long with ranges of 1 to 3 m, then lit only 1 to 3 m round their middles, and the shelves
      under them stayed dark.
- **A long lamp's beam (`Emission::Wedge`, 2026-11-06):** a bar lamp's beam is a wedge
  along its line, not a cone from its middle. The owner saw it on the private district's pool table,
  where a 3.1 m lamp lit a narrow cone. It is a tube's line in closed form times a cone's step in the
  angle from the way it faces to the line's nearest point, so its shape runs along the lamp with
  rounded ends and no lobes (a row of cones showed lobes at 0.75 m). Surfaces, ReSTIR's samples and the
  fog all take it, in the line's closed forms: the private district's source gives most of its long
  lamps a cone (`LT_Area`: 4,118 of about 5,000 at 160° or less), and without one a lamp over a table lit
  the wall behind it too. Other lamps are unchanged. A web search the same day found no
  patent on it; it adds a cone's step to the closed form below and takes no line light onto a plane.
- **Patents** (searched 2026-11-05):
  - **Pixar's US 7,872,649** (live to about 2029) projects a line light onto a plane through the
    shaded point. This shape is not taken: the line is integrated in space and clipped at the
    horizon.
  - **Sony's US 9,262,860** (equi-angular sampling along a camera ray in a medium) is not taken
    either: the fog takes the line's light in closed form.
  - **No patent found** on the closed form (1985, 1992) or on Epic's representative point (2013's
    published course notes).
- **A read-only review** found five faults before the push, all fixed:
  - the highlight lit by the whole line's peak at one point, up to 15 times ReSTIR's beside a strip;
  - shadow rays to the part behind the horizon;
  - the penumbra filter's rays bunched at the middle;
  - the fog's shadow ray to the middle;
  - the resolve without the denoiser treating it as a bulb.
- **The numbers:**
  - **The sign** (the private district's viewer): as with the flat pieces, a band along the strip
    by day and at night in place of one hot spot (above).
  - **The district's path** (2560 × 1440, DLSS Quality, interleaved after a warm-up): 16.30 and
    15.74 ms against 16.10 and 15.56 with the flat pieces and 15.88 and 15.30 with points, at a lamp
    a strip (`docs/PROFILE.md`).
  - **Its 14 frames against the source's:** the bar's landing −0.44 → −0.32 EV with the ranges from
    the line, the shelves under its strips lit; their rms 0.45 either way.
  - **The Bistro's night** (no tubes; its lanterns' test changed): the 16 × 9 tiles' mean |log2|
    0.000 against the build before, the frame ×1.000.
  - **The village's map-check views:** as before this work (every view's mean at most 0.002 from
    the accepted set, `enter`'s standing 0.046).

**The penumbrae's runs interleaved and spread (2026-11-07):** the filter's next step that
`research/lamp-penumbra-denoising.md` §7 named for penumbrae that still crawl (an à-trous pass at a
fixed step), with the frame's runs interleaved over 4 × 4 blocks.
- **What was seen:** in the private district's bar, a still camera's frames 10 apart changed on a
  shelf by up to 236 codes (2560 × 1440, DLSS Quality: 4,898 pixels by more than 30). The shelf
  stands a few centimetres under a neon tube with a row of bottles in front of it.
  - The probes were suspected first. Turning them off removed it, but only because the lamps'
    rays then have no rooms: the probes' maps held still at 0.999 changed nothing. The new probe
    debug view (`r.show.probes`: each probe's last update on screen, `Probes::set_debug`) and a
    dump of the room grids' probes showed every one of them settled.
  - Lamp shadows off (`CP_LAMP_SHADOWS=0` in the private viewer) took it to 972 pixels.
  - Eight rays a lamp, one to each of the cycle's runs, took it to 315.
- **The cause:** every pixel of a frame took the same run of 16 points along the tube. With
  bottles in front of it, each run got the tube's visible length wrong in the same way all over
  the shelf. The filter averages pixels, so it could not remove an error they all shared, and
  DLSS followed it from frame to frame.
- **The change** (`meshlet.slang`):
  - `lamp_disc_run`: neighbouring 4 × 4 blocks take different runs, all 8 in each 4 × 2 blocks.
    Interleaved sampling again (Keller and Heidrich 2001), one level up.
  - The strongest lamp's two runs stay an aligned pair, stratified as a whole: run r and r xor 1.
  - **"shadow/lamp spread":** after the 7 × 7 tent, 5 × 5 taps 4 pixels apart, a B3 kernel
    (1 4 6 4 1), the filter's own edge weights (the region, n·n′ > 0.9, the plane, the unshadowed
    light), after Dammertz et al.'s edge-avoiding à-trous (2010).
  - The step is fixed: no width from a hit distance, no variance, no history (the note's §3).
- **Tried and left out:**
  - **Interleaved without the spread:** 438 pixels, but a grain the size of the blocks stayed on
    the shelf (the crop's 3 × 3 contrast 6.94 against 6.33 with eight rays).
  - **The spread at the penumbrae only** (a share between 0.02 and 0.98): the lit pixels' grain
    stayed (579 pixels).
  - **Steps of 2, 3, 5 and 8:** no better than 4; 4 alone with no gate matched the eight rays.
  - **The spread without the interleave:** 4,469 pixels. A wider average of the same run keeps
    the run's error.
  - **The pick of a fourth lamp left out:** unchanged (4,900). Its rays were not the cause.
- **Checks** (the bar's shelf at 2560 × 1440, DLSS Quality, frames 220 and 230):

  | | pixels > 30 codes | largest change | the shelf's mean change | its 3 × 3 contrast |
  |---|---|---|---|---|
  | before | 4,898 | 236 | 6.33 | 6.04 |
  | eight rays a lamp (a reference, too dear) | 315 | 145 | 0.36 | 6.33 |
  | runs interleaved and spread | 315 | 140 | 0.46 | 6.42 |

  - **D-065's own scenes** (1600 × 900, a still camera's frames 210 and 220, pixels changing
    by more than 2 codes), before → after:
    - the lab's lamp room: DLAA 0.0036 → 0.0037 %, FSR 0.029 → 0.024 %;
    - the smoke yard: DLAA 0.101 → 0.102 %, FSR 0.247 → 0.247 %.
  - **The lamp room's frame** moved by at most 14 codes on 0.07 % of its pixels, at a shadow's
    edge: the spread softens a sharp shadow there a little. The smoke yard's is unchanged.
  - **Unchanged:** the guards (ast240, lab-creatures120, 0 px) and the village's map-check views
    (at most 0.002 % of pixels against the run before).
- **Cost:** "shadow/lamp spread" 0.14 ms at 1707 × 960 in the bar (`docs/PROFILE.md`). The rays
  and the filter are unchanged.

## D-066 — Pieces with sockets: one connector schema for generation, the anomaly check and player building ✅ (proposed 2026-10-16 on the owner's walk of #266; the owner's go the same evening)

**The question:** the owner, walking the village: "would using some kind of snap system with the
module parts help, if it does not snap everywhere, it means something is wrong, we need a way to
detect anomalies for building and probably roads, walls and other things"; "if one day we allow
players to build structures we would need a snap system anyway"; "the shape of the module piece
and its type in itself would authorize or reject what's possible to join it to"; "apply it to
sidewalks and roads as well". Research: `research/building-sockets.md` (how shipped games join
pieces, connector models, stability, checking, patents) and `research/house-layouts.md` (the
rules the village's houses, lots and walls should keep).

**What exists:** the kit's modules fill scopes the facade grammar cuts (`forge_geom::modules`,
`forge_procgen::grammar`); the structures, interiors, garden walls and the road graph are each
built by their own code; `city::anomalies` (the same evening) counts seven kinds of breach
without sockets (rooms past their walls, partitions in openings, unlit rooms, slivers, eave steps,
crooked walls, misplaced plants), and its test fails when a count rises.

**Proposed:**
1. **A piece schema with sockets** (`research/building-sockets.md` §6): each piece its family,
   sizing (fixed, or a scope's range), material, solids, faces and keep-out volumes, and its
   sockets: a kind, a profile (a cross-section: "wall 0.40 rubble", "eave 30° tile"), a frame on
   the scope's boundary, an extent (a point, a span, a rectangle: Space Engineers' mount points),
   a fit, a polarity, the turns allowed, a need (required, optional, an end cap), a bearing and
   veto masks. One `compatible(a, b)` for every user: a static matrix of kinds (Tessera's), equal
   profiles or a listed transition piece, opposite normals within 1° and positions within 1 cm.
2. **Sockets on what exists, in order:** the 16 kit modules (wall seams, foot and head, holes and
   their fills with thresholds), the structural walls and slabs, the roofs (eaves, ridges, hips,
   valleys, verges onto party walls), the garden walls (ends, gates), then the road graph and the
   pavements (road ends, kerbs, path ends with their slope).
3. **The checker on sockets:** every required socket matched exactly once by a compatible one,
   no mismatched pair, no double match, no overlap or coplanar faces of two pieces, nothing
   floating (connectivity from the ground anchors), openings against rooms (a door between two
   floors at one level, a window onto open air), networks matched; its counts by kind in the map
   check, an overlay colouring the sockets; `city::anomalies`' seven kinds kept beside it.
4. **Generation through it:** the grammar stays; its terminals make pieces with sockets, and a
   join pass places corners, end caps and transitions to meet the required ones, so a lot's
   corners, a row's party walls and roofs, and a garden wall's ends are made by matching, not by
   case-by-case geometry. WFC, through the same `compatible`, only for small discrete problems
   (roofscape clutter, chimneys, light wells).
5. **Player building later, on the same data:** free sockets in a spatial hash, candidate pairs
   within about 0.5 m filtered by `compatible` and cycled by hand (Valheim's), the ghost coloured
   by support, the same checker on each placement.
6. **Support:** connectivity first (floating pieces); support decaying a joint or a metre as a
   widest path from the anchors (Valheim's model in one pass) for the ghost; capacity flows later;
   stress only offline.

**Patents** (`research/building-sockets.md` §5): none found on typed sockets for building pieces
or on Valheim-like support. Live and to avoid: Nintendo's US 12,440,766 B2 (bonding free objects
at marked positions while one moves: no carry-and-glue builder, no vehicle or contraption
builder without the owner's word) and US 12,667,784 B2 (detaching an assembly's members);
Sony's US 8,152,613 B2 (stress nodes breaking over a threshold, to about 2030: no stress-and-break
pass without the owner's word); NCSOFT's KR 10-2310107 B1 (WFC on generated node graphs, Korea:
WFC only on lattices or grammar slots).

**Enshrouded,** which the owner pointed at: voxels with prefab shapes and terraforming, no
structural integrity or building physics found (its 1.0 changelog of 2026-10-15 included): its
lesson is freedom on a voxel base, for the terrain and the gardens, not the checker.

**For the owner to decide:** the schema and its order (1–3 first, then 4; 5 and 6 later); and the
two live Nintendo patents' reach before any free-form builder.

**Decided** (the owner, 2026-10-16): "D-066 ok let's see what it would look like, we still wish to
keep a bit of organic feeling but let's see the result of what you propose, we may adapt things
later. so yes." The lanes keep their curves; the sockets decide how the pieces meet. The same
answer set two rules for the dwellings: no lifts in houses (their stairs stay; some houses are
single-storey ones suited to disabled people; stair lifts out of scope), and living quarters no
higher than medium density, the smaller courtyard blocks two storeys at most.

**Built** (the same night): the schema's first slice (`forge_procgen::sockets`: kinds, points and
faces, `compatible`, `match_all`) on the village's houses, garden walls, gates and hedges, its
unmatched sockets as three new kinds in `city::anomalies`; generation through it for the houses
(rows of one depth, a 12 m depth cap, party walls on the lot lines, the interiors fitting slanted
walls). `demos/city-plan.md`, "Sockets and rows", has the numbers. Next: the kit's modules' and
the roofs' sockets, the garden walls' join pass, the pavements and roads.

The kit's openings, the second slice (2026-10-17): each structure's opening a hole, the module
the facade rules put in it its fill (`facades::fills`). 92 windows misfit their modules: the
balconies' French windows, drawn from 0.1 m over the floor on windows with a 0.9 m sill, walled
up inside. The windows the rules shape are fitted to their modules (`facades::fit_windows`),
and `OpeningMisfit` counts the rest (0). The doors' thresholds the same day: 11 doors opened onto
a garden wall (`DoorBlocked`); stretches along a house's wall nearer than 1.2 m are no longer
walled, their freed ends joined to the house, and a door a wall ends against goes back to a
window (0 left). The facade modules already tile their walls (369 storeys, no gap, no overlap).
Next: the roofs' sockets, the pavements and roads.

## D-067 — Local exposure: a bilateral grid of log luminance, built and used in one frame ✅ (proposed 2026-10-17, #275; the owner's go the same day)

**The question:** the owner, walking into a village house: "light outside is very strong, I would
expect the inside to be a bit brighter, maybe it's a fix in your roadmap later ?". D-022 meters
one exposure for the frame, keyed on the log-average of its 50th to 98th percentiles, so a sunlit
doorway in view sets it and the hall falls stops under middle grey. Research:
`research/local-exposure.md`.

**Proposed** (`research/local-exposure.md`, "Recommendation for Forge"):
1. **A local exposure before the display transform.** The resolved HDR image (TAA's or DLAA's)
   is splatted into a 64 × 36 × 64 bilateral grid of log luminance, blurred, and blended 60/40
   with a wide Gaussian of the cells' means (Ghost of Tsushima's, Patry 2021). Each pixel reads
   its base luminance B at its own luminance and takes the gain
   `2^((c − 1)(B − M))`: c = 0.7 under middle grey M, 0.85 over it, clamped to +3 EV and −2 EV,
   and none in near-black.
2. **Passes:** `post/local exposure grid` and `post/local exposure blur` (compute), and the read in
   the display pass. The histogram, TAA, DLSS and bloom stay as they are. Estimated 0.04–0.08 ms
   at 1600 × 900 and 2.4 MB, to be measured (budget 0.1 ms).
3. **Off by default until the owner has seen it**, then on in the village and the city.
   - Controls: `--local-exposure`, `--local-exposure-contrast S,H`, a key, a line in the F1
     overlay, and a false-colour view of the gain.
4. **Checks:**
   - CPU unit tests of the read and the gain;
   - the map check's hall views, plus a new view looking at the open front door from the hall;
   - halos with ꟻLIP against the global-exposure frame in bands along the door's edge;
   - gradient reversal on a sky view;
   - flicker on a still camera under DLAA and on the way-in walk;
   - the cost in the F1 zones;
   - one AMD check.
5. **Left for later:** exposure fusion (the alternative if the grid rings), detail strength other
   than 1, contrast by EV100, any temporal filtering of the grid.

**Patents** (`research/local-exposure.md` §6; not a professional search):
- **Expired:** the foundational patents. MIT's US 7,146,059 B1 (Durand and Dorsey) and the
  NXP-line bilateral-grid family lapsed for fees.
- **Nearest live one:** STMicroelectronics' US 11,756,172 B2 (active to 2041), which claims
  modifying a later frame with a low-resolution log-luminance image of an earlier one. Forge
  builds and uses its grid in the same frame and keeps no grid history.
- **Other live ones:** camera pipelines with per-region tone curves (Apple, Texas Instruments),
  GoPro's sensor chain, Irystec's analytic contrast-distortion curve and Dolby's multiscale ratios.
  None of them is the proposed method.

**For the owner to decide:**
- the method and its defaults;
- whether STMicroelectronics' patent's reach is acceptable with the grid kept within one frame;
- whether to build it now or after the lamps of D-065, which change how bright the halls are by
  themselves.

**Decided** (the owner, 2026-10-17): "D-067 go with recommendations". The method and defaults as
proposed, built now, off by default until the owner has seen it; the grid built and used within
one frame, keeping clear of STMicroelectronics' claims. Its patents' details and conditions of
application are to be looked into further (D-068's note; done in `research/lighting-patents.md`).

**Built** (2026-10-17, #275). As proposed, off by default.
- **Code:** `crates/forge-render/src/local_exposure.rs`, `shaders/local_exposure_grid.slang`
  (the passes) and `shaders/local_exposure.slang` (the read).
- **Where it is read:** in TAA's show pass and in the display transform, after bloom is mixed
  in. The night's Purkinje shift stays on the scene's own luminance.
- **Middle grey M:** where D-022 keys the scene, `log2(2^compensation / 9.6)`, less the stops
  the night is shown darker by.
- **The layout:** the grid's 64 slices lie in a 512 × 288 RG16F atlas, so a trilinear read is
  two bilinear taps.
- **The patent rules**, from the village session's reading of the US claims and EP 3872750 B1:
  - no grid history;
  - no strength as a mix of the gain: the near-black fade scales the gain's EV;
  - no stored gain map: each pixel computes its own gain in the display pass.
- **Controls:** `--local-exposure`, `--local-exposure-contrast S,H`, `--local-exposure-view`,
  the X key (off, on, the gain view) and a line in the F1 overlay. A hidden
  `--local-exposure-blend` exists for measuring.
- **The map check** gains a `door` view: from the back of the hall, looking at the open front
  door.

**Measured** (RTX 5070 Ti, 1600 × 900; `docs/demos/city-plan.md`, `PROFILE.md`):
- **Cost:** 0.062 ms under TAA, of which the read is 0.031 because RCAS reads five pixels;
  0.042 ms under DLAA; 0.126 ms at 1440p under TAA. About 1.2 MB of transients.
- **The walk in:** at its darkest frame, the frame's mean display luminance is 2.0 EV higher.
  It adds no frame-to-frame step over 0.05 EV.
- **Once the exposure has adapted:** the door and hall views move by a few tenths of an EV.
- **Halos:** ꟻLIP bands beside the door read 0.86 near/far, against 0.83 for the grid alone.
- **Gradient reversal on a low-sun sky:** as many pairs turn over as with a uniform +0.5 EV
  change of the global exposure (7 847 against 7 662). Contrasts of 1,1 give the same image to
  the pixel.
- **Still-camera flicker under DLAA:** 0.031 % against 0.038 % without (door), 0.011 % against
  0.012 % (hall).
- **Unit tests:** the CPU mirror.
- **Validation:** clean under synchronisation validation with local exposure on: TAA, DLAA, the
  gain view and the fallback path.
- **AMD:** one run of the physics lab's yard with local exposure on, on the dev machine's
  RDNA 2 iGPU. It ran clean: the grid took 2.19 ms and the blur 0.28 ms, in a 120 ms frame.
- **With it off** (the default): Tier 1 matched the accepted set (only #71's flake), and the
  map check's views matched to the pixel.
- **Left for the owner:** the look. Then on by default in the village and the city
  (`map-check --accept`).
- **On in the village** (#275, 2026-10-21). The owner saw it on 2026-10-19 in a scene built
  outside the repository. From a lit hall, an open door showed a windowless room pitch black
  without it ("makes no sense"); with it, dim, as the eye sees it. That scene measured the meter
  as the cause: at a fixed EV 6 the room is only a bit darker than the hall, but the band
  (0.5, 0.98) let the hall's lit walls alone set the exposure. A band of (0.25, 0.95) and a
  shadow contrast of 0.55 made the room read "a bit darker", as the owner expects, and barely
  changed a street view. A scene's look now turns it on with those two (`Look::local_exposure`;
  the village's, with its metered exposure; `--no-local-exposure`, `--meter-band` and
  `--local-exposure-contrast` override). In the village's map check the streets keep their light
  and the trees' crowns open up; the hall's doorway is less blown.

## D-068 — Probe regions from the generator: a room's probes light only its own surfaces ✅ (proposed 2026-10-17, #266; the owner's go the same day)

**Amended (2026-10-31, #378): a reveal met from outside is the outside's.** A ray's hit took its
region 2 cm off it (`probe_irradiance_hit`), so the traced bounce's rays from a facade that met a
window's reveal (the room's, by the rule below) took the room's probes: at night a lit room's light
came back onto the wall round its window, a warm halo 1.5 times the window's size (the owner: a
spotlight, "a nuclear plant powered bulb"). Unchanged with the bloom, the anti-aliasing, the lamp
filter, the far rooms' lamps, the room grids, local exposure or the bounce's rays stopped at glass;
gone with the probes or the bounce off, and with every hit taken as outside. Now a hit in a reveal
takes the region 30 cm back towards where its ray came from (one `region_at` in a rolled loop,
#348): from outside the outside's, from its room the room's. **Not the fix (measured the same
evening):** the wall round the window reads the same with it (0.0474 of white in a 12-pixel ring,
anti-aliasing and bloom off), against 0.0289 with every hit taken as outside. On the village's
stone walls (0.6 m, the air before the glass inside the house's body) a ray from the street meets
the reveal deep enough that 30 cm back is still the body, and the rule never switches; treating
that as outside made the ring brighter (0.0537): some of the light comes from rays that start in
the opening itself, the window's own pixels, which REBLUR then spreads. Hits taken as room-kind
(not a reveal) carry about half of it (0.0348 with those alone made outside). #378 stays open.

**The question:** after the village's probes moved to 1 m (ef27049) and the lookup kept the
probes in front of a surface (22beb20), light still leaks indoors in two ways: a soft tinted
blotch on a partition from the sunlit room behind it, and a small bright cap in a window head's
reveal. Both stem from the owner's "lighting still seems a bit strange to me in corridor with the
stairs" and "a lot of lighting issues a bit everywhere inside the house". Research:
`research/probe-leaks-indoors.md`.

**What exists:** DDGI-style probes (D-036, `probes.slang`), with relocation, classification, the
Chebyshev visibility test and the 2021 paper's bias, as RTXGI does. The 2022 paper and RTXGI's own
docs name thin walls as DDGI's open case. The village's generator knows every room's outline,
every opening and every stair hole.

**Proposed:**
1. **Regions from the plan.**
   - Each house gives its rooms (an outline with floor and ceiling heights, one region number
     each) and its openings (a box through the wall or slab, the two regions it joins, glazed or
     not).
   - `region_at(p)` finds the room, else an opening's recess, else the walls' solid, else the
     outside.
   - A probe takes the region of its relocated position while young; a probe in a wall or a
     recess takes none.
   - A surface takes the region in front of it (`p + 2 cm · n`).
   - In the lookup, a probe weighs 0 on another region's surfaces. Open doorways, stair holes
     and unglazed windows ramp each side's probes across their depth, so the jambs shade into
     both rooms.
   - Where no probe of the region is among the eight, the next cascade is tried, then the
     unmasked lookup; the fallbacks are counted in the F1 overlay.
   - Estimated 0.02–0.06 ms and 166 KB; no subgroups, no groupshared memory.
2. **Two near-free fixes with it:**
   - back-face hits stored at a fifth of their distance (RTXGI; the NVIDIA application on DDGI was
     abandoned in 2023);
   - the interpolated normal for the bias and the back-face weight, keeping the bent normal for
     the irradiance's direction (#70).
3. **Checked by** unit tests of `region_at` on the kit's walls, reveals, jambs and stair holes,
   the map check, three new views (a partition with the sun behind it, a window head from inside,
   a doorway), the leak's mean log luminance with regions off and on, the still-camera measure
   (0.05 % under `--dlaa`), F1 timings and `tools/amd-check.sh`.

**Patents** (`research/probe-leaks-indoors.md` §4; both records checked again on 2026-10-17):
- **Square Enix, US 10,137,372 B2** (EP 3306569 B1, JP, CA), active to 2037-02-01. It claims
  culling probes by zones made of occlusion planes predefined for each probe, a per-probe mask
  and a bitwise test. Forge's regions would come from the plan, one number per probe and per
  surface compared for equality, with no per-probe planes or masks. It is the nearest live patent
  to the proposal, so the owner decides.
- **Imagination Technologies, US 10,008,032 B2** and its continuations (GB 2571656 B), active to
  2036-08-26. Claim 1 describes probes whose directional lighting is updated each frame by
  tracing rays from them and then used to shade. That reads on Forge's probes as they are
  (D-036, built before this was found), not on the regions. A professional opinion would settle
  its reach.

**For the owner to decide:**
- whether to build the regions, given Square Enix's patent;
- what to do about Imagination's patent on the probes already in the engine (an opinion, or
  living with it until a sold product);
- whether the two near-free fixes can go in first on their own.

**Decided** (the owner, 2026-10-17): "D-068 go with recommendations, Square enix patent does not
seems to concern us, so build the regions. The imagination one seems too broad and would
appropriate the ray tracing technology itself. so let's live with it for now. And use the free
fixes. Add a note to try to get more details on those patents. And what are the conditions of
application of their patents just in case."
- The regions are built as proposed, with the two near-free fixes.
- Imagination's US 10,008,032 B2 family: lived with for now.
- **To look into** (the owner's note): more details on these patents and the conditions under
  which they apply (the claims' exact elements, the jurisdictions they hold in, which acts
  infringe there, their status, fees and expiry, any licensing), Square Enix's, Imagination's
  and STMicroelectronics' (D-067) first, in a research note before any sold product. Done the
  same day: `research/lighting-patents.md` (not legal advice).
  - Imagination's independent claims read on Forge's probes as built.
  - Square Enix's US claim 37 (zones and "a grid of regions", no occlusion planes) is the claim
    nearest the regions, a question for counsel. The regions keep to one number per probe and
    per surface compared for equality, with no per-probe zones, masks or bitwise test.
  - STMicroelectronics' claims all reuse one image's mini-image on a second image; D-067's grid
    is built and read within one frame.

**Built** (2026-10-17, #266). As proposed, with the changes found on the way below.
- **Code:** `probe_regions` in `demos/city-plan/src/lib.rs` (the volumes, from the houses whose
  insides are drawn), `Probes::set_regions` and `region_at` in `crates/forge-render/src/probes.rs`
  (the GPU's lists and the lookup's CPU mirror), `region_at`, `cascade_irradiance` and
  `probe_lookup` in `shaders/probes.slang`, the region's write in `probe_update.slang`.
- **The volumes:** a room and the hall (its stair well included) per region; a doorway from one
  side's region to the other's, its ramp across the partition's thickness; a window's inner
  reveal, from 6 cm before the glass (its outer face stays outside's) to the room, its room's; a
  house's footprint, last, lit by every probe as before. About 1 350 in the village, in a grid
  of 2 m cells (6 a cell on average, 17 at most).
- **Windows let the daylight in.** With regions, an upstairs room seen through the landing's
  doorway went black. A window's glass is drawn from outside only, and a probe's ray that met it
  from behind ended there: the rooms had only ever had their doors' light and what leaked in. A
  room's probe's ray now passes the glass of a glazed row (`RenderLayer::glazed`, implied by
  `interior`: a home's, a shop's, a bathroom's frosted glass) met from behind. Only a room's
  probe: in a house without insides (a hollow shell) the ray still ends there.
  - **The clear glass of a house whose rooms are drawn** (drawn after the resolve, D-065) is
    passed by every probe's ray, either way (#280, 2026-10-19).
    - **Why:** met as a back face, a pane counted a probe by its window as inside something.
      The probe was moved out through the glass and lit its room with the street. The probes
      outside saw a lit pane where the room is.
    - **What it changed:** a bedroom whose storey has no regions (lit by every probe) had been
      2.7 EV darker without its glass than with it. Passing the glass in each kind of ray found
      it: the probes' own rays alone (the fixed rays 0.76 EV, the light-gathering ones 0.40),
      and from the probes without a region alone. The sun's, the lamps' and the mirror rays and
      the probes' hits' shadow rays changed nothing.
    - **Now:** that bedroom is 1.78 EV darker at a fixed exposure, the same with or without its
      glass. The map check's landing (the same storey) is 0.28 EV darker at its metered
      exposure, the other views within 0.03 EV. The lab's rooms and the city are unchanged (0 px).
    - **The shadow rays pass it too** (the same day): the sun's and the lamps' rays, the probes'
      hits' included (`shadow_candidate_kept`). The glass is a candidate section in its mesh's
      structure (#307: `GpuMaterial::clear_glass` counts as masked), so the other rays still meet
      it at no cost. Before, a pane shadowed its own reveal and the floor behind it: in the
      evening (`--time-of-day 0.85`) the room's window jamb and table now take the sun. The map
      check at 15:00 changes by ꟻLIP 0.002 at most; the day check is 0 px.
- **A probe outside every house** (most probes, the coarse cascades' too) lights what its rays
  meet as the outside's, without the lookup. The lookup at every ray's hit cost the rays
  0.14 ms in a room.
- **A ray's hit takes its region's probes alone** (`probe_irradiance_hit`): no fallback to every
  probe, but no bounce in a house where none of them stands near, and the open sky outside; only
  the shading keeps the fallback's second copy of the lookup.
- **A room's cascade claims only what its probes cover** (#283, 2026-10-18).
  - **What was wrong:** a window's head showed a dark trapezoid with a hard diagonal edge, by day
    and at night. Few of the room's probes stand near a recess. Where a cell boundary crossed the
    head (the house at an angle to the grid), the one room probe among the eight changed, and
    each alone was all the light.
  - **The rule:** a room's cascade covers a point by its own probes' share of the trilinear
    weight, up to `PROBE_ROOM_COVER` (0.25), and the next cascade fills the rest. What the room's
    cascades leave is shared among them, never taken from the open sky, so the outside's light
    still stays out.
  - **Where it applies:** not outside, nor in a scene without regions; there nothing changes.
  - **Cost and result:** the window's head is even. The map check's views moved by at most
    0.010 mean ꟻLIP (the room's window frame) and 0.0015 EV. The resolve costs about 0.09 ms
    more in a room (`shading/standard` 0.83 → 0.92 ms at the recess's view at night), from
    evaluating the next cascade near walls.
- **The lookup's cell is clamped into its grid before its read** (f7145b1), and a scene without
  regions names a buffer of zeros: unclamped, the physics lab's dominoes at frame 900 differed
  from run to run by 20–40 scattered pixels (Tier 2 caught it on both paths, async and serial),
  though the lab has no regions and the lookup returns before its read there. The GPU read ahead
  of the return, far outside the grid. A loop form of the fallback made it more frequent, and
  was set aside for it at first.
- **No count in the F1 overlay:** `--show-regions` shows where the lookup fell back (red), with
  each region's colour, the reveals green, the houses' bodies white and the outside black.
- **The checks:**
  - unit tests of `region_at` (rooms, a doorway's ramp either way round, a reveal, the body,
    outside), of the grid against the plain lookup, and on the village (every room's middle, every
    reveal and doorway: 408 rooms and halls, 472 reveals, 466 doorways);
  - the leaks gone in the rooms that showed them (33-0's blotch, 0-1's band, 0-0's cap), the map
    check's room (a window head from inside) and landing (a doorway) standing for the three new
    views;
  - timings (the map check's views at its hour, 240 frames, best of three, c963e68 against D-068):
    the room 3.67 → 3.85 ms, the hall 4.05 → 4.26, the green 4.40 → 4.57, the landing 4.19 →
    4.39, nearly all in `gi/probe rays` (+0.14 to +0.18) and `shading/standard` (0 to +0.03);
    the city (no regions) 3.02 → 3.07 ms. The first build cost the room 0.28 ms: 0.14 ms the
    lookup at every ray's hit (skipped now for a probe outside), 0.03 ms the rays through the
    glass, about 0.1 ms a second copy of the lookup for the fallback, which a ray's hit now does
    without;
  - a still camera under `--dlaa` (frames 10 apart from 200): the room 398–674 changed pixels
    of 1.44 million (341–642 before), the hall 113–253 (125–248), the landing 11–26 (3 000–41 000:
    it had not settled), room 33-0 967–1 545 (745–1 029), all of them in the trees seen through
    its window;
  - the map check, re-accepted: the rooms and the landing lit by their windows (ꟻLIP mean 0.31 and
    0.28 against the old set), the streets within 0.023.
- **Not done:** `tools/amd-check.sh` (it draws no probe regions).
- **Its effect:** the rooms are lit by their windows, brighter than before (the owner asked for
  it: "I would expect the inside to be a bit brighter").
- **Rooms beyond the room-sized cascades** (#321, 2026-10-19, found by a scene built outside the
  repository): from about 10 m in the village only cascades coarser than `PROBE_ROOM_SPACING`
  cover a room, and at 4 m spacing the probes stand at about 2, 6 and 10 m: an upper storey
  (3.0–5.7 m) holds none of its region's. The fallback to every probe then mixed other rooms',
  the roof's and the street's probes behind walls and slabs, and such rooms came out black
  through their windows until the camera was within a few metres. Now a room that no room-sized
  cascade covers and whose region has no probe among the eight takes `ProbeParams::far_room_sky`
  (0.06; `--probe-far-sky`, or the scene's `Look::probe_far_room_sky`: about 0.27 for shallow
  rooms with wide windows, bracketed out of the repository) of the open sky's luminance, what its windows let in (a daylight
  factor, deep rooms behind small windows less, shallow ones with wide windows more), in a warm
  white (`far_from_rooms`; `--show-regions` tints it orange). A first version took 0.25 of the
  outside's probes around it: in a deep house under a pitched roof those include the roof
  void's (no region covers it, so its probes count as outside), and the upper rooms got a
  fifth of what the share promised, still near black (the scene built outside the repository:
  facade 0.136 linear under `--show-gi`, its upper windows 0.007). The sky's own light, unlike
  the probes, does not depend on where they stand; its blue turned the floors blue, hence the
  luminance and the tint. Calibrated by forcing the fallback on the village's room and hall,
  which have their own probes: in linear light 24.6 against 22.7 and 16.7 against 18.4 at 0.06
  (0.15 made them 2–2.5 times too bright). Near the camera nothing changes: the map check's
  interiors 0 px, its street views at most 0.0023 mean LDR-ꟻLIP against the first version.
- **A doorway shut by its door** (#327, 2026-10-21): a doorway's ramp assumes an open opening,
  and a shut door's outer face took the dark room's probes (blotches on a garage door in a scene
  built outside the repository). `Probes::set_doorway(volume, Some(s))` puts the leaf at `s` of
  the way across: each side of it takes its own side's probes alone (`RegionVolume::split`);
  `None` opens it again. From the next frame, without starting the probes over: once a doorway
  is set, each frame slot reads its own copy of the volumes, rewritten after an edit, as the
  glass panes' transforms (#325). A scene hands its doors' states through
  `Running::doorway_edits`. With no doorway shut, the map check is unchanged.
  - **Either way round** (2026-10-22): the lookup turns a doorway given clockwise round, which
    swaps its sides, and its leaf's share was not turned with it: the leaf stood mirrored
    across the doorway, the door on the far room's side, black on that scene's garage doors (a
    ninth of its doorways). `set_doorway` now measures it from the side the lookup keeps
    (`split_as_stored`), tested both ways round.
- **No pop as the room-sized cascade reaches such a room** (#334, 2026-10-21, the owner's
  playtest of a scene built outside the repository: "there is a pop … a window thing again").
  The room-sized cascade fades over one spacing at its edge, and where it covered a room partly
  its share was taken for all of the room's light: walking towards a building, its windows
  switched from the far room's share to the probes' within a metre. Now a room's surface within
  `ROOM_BLEND` (4) spacings of that cascade's edge across the ground, whose light beyond it would
  be the far room's, is blended from that into the probes' light over the band (`room_depth`,
  `probe_lookup_with` in `probes.slang`). Where a coarser cascade holds the room's probes nothing
  changes. The map check moved by 7 px.
  - **Where the coarser cascade holds the room's probes too** (#353, 2026-10-22, the owner in a
    scene built outside the repository: "the wall lights up if you move forward"). The 2 m
    cascade lit rooms darker than the 1 m one, and a far wall brightened within a metre as the
    finest cascade reached it.
    - **The cause, measured:** a still village room, lit by the 1 m cascade, then by the 2 m
      alone (the cascades drifted away). Most of the 12.7-level gap on its back wall came from
      the 2 m probes' own bounce. Their rays' hits were looked up 0.36 m towards the probe that
      shot them (the view term of the lookup's offset, which grows with the spacing). Without
      it, the 2 m cascade's walls rose 5–8 levels and the 1 m cascade's under one.
    - **What changed:**
      - a hit in a room takes the offset along its normal alone (`probe_irradiance_hit`); the
        outside's hits keep both terms, so outdoor light is unchanged to the bit;
      - a room's surface leaves a room-sized cascade for the next over `ROOM_BLEND` spacings,
        not one (`cascades_lookup`).
    - **Measured:**
      - the village room's back wall gap went from 12.7 to 8.5 levels;
      - drifting the cascades towards a house door at 1.4 m/s, its dark hall rose 24 levels
        over 2.8 m, up to 5 a 0.3 m; now 19 over 4 m, 1.4 a 0.3 m, from 6 levels higher.
    - **The map check:** interiors moved by ꟻLIP 0.016–0.043 (the landing's quadrants by at
      most 1.4 luma), outdoor views under 0.001.
    - **Left:** what remains is how a 2 m grid interpolates a room (a table top 13 levels
      darker, a side wall 13 brighter). The issue keeps those leads.

## D-069 — One set of engine settings for every demo, defaults at the best look, and a console on ² ✅ (proposed 2026-10-17, #278, the owner's request; the owner's go the same day)

**The question:** the owner asked, on 2026-10-17:

> "Could we backport the changes over the demos and make it overall consistent? And make a in
> engine dev console activable with the key ² and with autocompletion like most of the game
> engines do. And use the flags only when we do something outside of base behaviour for now?"

**What exists:**
- Each demo parses its own flags with clap. Asteroids has about 40, planet about 30, meshlets a
  few, and city-blocks the most; physics-lab, island and city-plan are built on city-blocks.
- The same setting is spelled or defaulted differently from demo to demo. `--dlaa` exists only in
  city-blocks, `--no-taa` in three demos. Some of the best settings are flags rather than
  defaults.
- Runtime switches are single keys per demo (F1–F6, N, Y, …), with no names and no list.
- Scripts depend on today's flags and defaults: `tools/captures.sh`, `tools/map-check.sh`, the
  timings and the sandbox's day check.

**How engines do it** (from what is known; the research note below will check it):
- **id Tech (Quake, 1996):**
  - a console on the key under Esc;
  - console variables with a name, a value and flags (saved, cheat);
  - commands, Tab completion, `+set` on the command line and `autoexec.cfg`.
- **Source:**
  - `ConVar` and `ConCommand`, registered where they are used;
  - flags for cheats, saved values and development-only variables;
  - `find`, `help`, candidates listed while typing, and history.
- **Unreal:**
  - console variables named by system (`r.`, `sg.`), with `-ExecCmds` and ini files;
  - a priority for who set a value (code, ini, the console);
  - completion with the help text beside each candidate.

**Proposed:**
1. **A settings registry in `forge-app`:**
   - Each setting is a console variable: a typed value (bool, integer, float, one of a list, or
     text). It has a dotted name (`r.aa`, `r.probes`, `r.ao.radius`, `app.fixed_step`), a default,
     a range, a line of help, and flags: debug only, needs a restart, saved.
   - The engine's systems and each demo register their own.
   - Systems read them every frame, so a change applies at once unless it is flagged for a
     restart.
2. **Flags only to step away from the defaults:**
   - Defaults are the best look: what the showcases use today.
   - Any variable can be set from the command line (`--set r.aa=taa`) or from a `forge.cfg`.
   - Today's flag spellings stay as aliases of the variables during the move, so scripts and
     the accepted captures keep working. They are removed once the scripts use the new names.
3. **The same settings in every demo:**
   - The shared ones move into the registry with one name and one default each: anti-aliasing,
     tone map and HDR, exposure (with local exposure), shadows and their denoiser, probes, AO,
     reflections, clouds, fixed step, frames and captures, the monitor, and the debug views.
   - Each demo keeps only its own scene's options.
   - Then the village's gains go to the other demos where they apply: DLAA by default, local
     exposure, glass the probes' rays pass, and the probe settings. Probe regions only where a
     scene has rooms.
4. **The console:**
   - **Opening:** the physical key under Esc (² on AZERTY, ` on QWERTY, winit's
     `KeyCode::Backquote`).
   - **Drawing:** with the F1 overlay's text over the top third of the window. While it is open,
     keys go to it and not to the camera.
   - **Completion:** Tab completes the longest common prefix and lists the candidates: names,
     then a value's choices. Each candidate shows its help and current value.
   - **History:** Up and Down recall it, and it is kept between runs.
   - **Built-in commands:** `help NAME`, `find WORD`, `list`, `reset NAME`, plus commands
     registered like variables (a capture, a view to jump to, the debug views).
   - **Feedback and safety:** each change prints what it did. Debug-only variables are marked,
     so a shipped game can hide them.
5. **Order:**
   1. the registry;
   2. the shared settings moved across the demos, with the scripts and one re-accepted capture
      set;
   3. the console;
   4. the backports.

   About four sessions in all.

**Checks:**
- Unit tests of the registry: parsing, ranges, aliases and completion.
- Each demo's captures before and after the move:
  - unchanged where only a name changed;
  - named with `--expect` where a default changed, with their ꟻLIP numbers.
- The console's cost in the F1 overlay: nothing while closed.

**The owner's answers (2026-10-17), with the go:**
- **Names:** dotted, "so that it's easier to use auto completion and find instruction by
  hierarchy".
- **Persistence:** console changes are temporary: "don't persist changes for now". `forge.cfg`
  is a file the owner or Claude edits by hand. Some commands may become saved later.
- **Release builds:** the console stays, "in a very limited scope to prevent cheating".
  - It may show information only: frame time and stats, as Star Citizen's `r_displayInfo` does
    (0 off, 1 on, 2 verbose), and the build's version in the console itself.
  - Nothing that gives an advantage. Every other variable and command is for development builds
    only.
- **Patents:** none to search for this.

**Research:** `research/dev-console.md` (2026-10-17, done): id Tech, Source, Unreal, CryEngine,
O3DE and the Rust crates; completion and history; release builds; winit's text input. What it
changed in the build: Tab steps through the candidates once their common part cannot grow; the
scripts never read the owner's `forge.cfg`; a misspelled `--set` fails a scripted run; names are
case-insensitive; `changed`, `toggle` and `NAME ?`. What it advises for a shipped game, later:
development settings and the code that would help a player compiled out rather than hidden, and
"information" kept to the machine, the build and the session (`docs/demos/console.md`).

**Built so far (step 1 and the console, 2026-10-17):**
- `forge_app::settings`: the registry (typed variables with dotted names, ranges, help, the
  release, restart and read-only flags, commands), `forge.cfg`, `--set`, `--cfg`, `--no-cfg`
  and `--exec` in every demo (`SettingsArgs`, flattened into each demo's arguments).
- `forge_app::console`: the console on the key under Esc, drawn with the overlay's text: Tab
  completion and cycling, the live list of what fits, the history kept between runs, `help`,
  `find`, `list`, `changed`, `reset`, `toggle`, `version`, `clear`.
- The shell's own: `r.display_info` (0, 1, 2) drives the F1 overlay, `app.console` shows the
  console, `app.capture` and `app.quit`; demos register theirs in `Demo::register` and run their
  commands in `Demo::command`.
- `shipping` (a forge-app feature) keeps only the release settings.
- Tests: 35 in forge-app (parsing, ranges, the order of the sources, completion and cycling,
  the file). Captures unchanged (Tier 0: every image as accepted but #277's dominoes).

**Built so far (step 2, 2026-10-17):**
- The registry lives in `forge-core::settings`; `forge_render::settings` names the renderer's
  shared settings once (`r.aa`, `r.dlss.mode`, `r.taa.sharpen`, `r.tonemap`, `r.bloom`,
  `r.exposure.*`, `r.ao`, `r.probes`, `r.sky_light`, `r.shadows`, `r.reflections.*`,
  `r.sun.disc`, `r.translucency`, `r.cull.*`, `r.lod`, `r.sw_raster`, `r.show.*`).
- Every demo registers those it has: the galleries (city-blocks, the island, the physics lab,
  the city plan), the asteroids, the planet, the meshlets bench. Each registers with its own
  default where its scene wants another look. Its flags set the settings as from the command
  line. Its keys set the settings, and the settings that changed are applied each update.
- A flag and its `--set` give the same image to the pixel in every demo checked (`--no-ao`,
  `--tonemap`, `--soft-shadows`).
- **Left for the owner:** the keys still differ between demos (P, X, V, Y, U, F;
  `docs/demos/console.md`): one key map for all of them is the owner's call.
- **Next:** the backports (step 4): what each demo lacks of what the others have.

**Step 4, the backports (2026-10-17, started):**
- **The planet anti-aliases with DLAA where it runs**, as the galleries have since D-045: the
  default of an interactive run; scripted runs keep TAA unless `--set r.aa=dlaa`, so the
  captures are unchanged. T steps DLAA, TAA, off.
- **The planet follows the window's size** (Alt+Enter): it had no resize, and kept its first
  targets.
- **The galleries' DLAA display follows the HDR output** each frame, as the asteroids' did: F2's
  switch reached only TAA's pass.
- What each demo has and lacks of the shared look, now that the settings name it:

  | | DLAA | DLSS modes | local exposure | probes, regions, glass | metered exposure | bloom key |
  |---|---|---|---|---|---|---|
  | galleries | yes | no | yes | yes | city plan, by day | B |
  | asteroids | yes | yes | no | no | yes | B |
  | planet | yes (new) | no | no | no | `--auto-exposure` | no |
  | meshlets | no | no | no | no | no | no |

  Local exposure in the asteroids and the planet waits for the owner's look at it (it is off by
  default in the galleries, D-067); the meshlets bench is a test of the culling, not of the look.

**The keys (the owner's answer, 2026-10-17):** "Unify the key map […] I would transform those
keys in console commands maybe. And let the keys free to be used for the user in game
interaction like F for interacting with doors. Z to go forward, S backward and so on." Keyboard
layouts other than the owner's AZERTY and QWERTY, with their own default maps, are low priority.
- **Developer toggles leave the keys.** What a key switched is a setting (most already are) or
  a command, set from the console. No key is bound to one by default.
- **`bind KEY LINE`** runs a console line when a key is pressed, as id Tech and Source do;
  `unbind KEY` and `binds` list and remove them. Binds typed in the console last for the run;
  `bind` lines in `forge.cfg` give the owner's own keys at every start. A key is named by what it
  types in the keyboard's layout (`t`, `z`, `²`) or by its name (`f5`, `tab`, `pageup`). In a
  shipped game a bind reaches only the release settings, as the console does.
- **The shell keeps** the key under Esc (the console), Esc, F1 (`r.display_info`) and Alt+Enter.
- **The game's keys stay the demos'**: the camera's movement reads the keys' places, not their
  letters (Z, Q, S, D on AZERTY are W, A, S, D on QWERTY), and the lab's, the island's and the
  city plan's walking, jumping and throwing keys stay. Named actions a player rebinds
  (`move.forward`, `interact`) come with the first game.

**The key map, built (2026-10-17):**
- **No developer keys:** the galleries', the asteroids', the planet's and the meshlets bench's keys
  are gone. What they switched is a setting, except F9 (`camera.where`).
  - New settings for what had a key and no setting: `r.shadows.denoiser` (F6), `r.sky.real` (the
    period key), `r.show.liquid` (1 to 3) and `meshlets.freeze` (F).
  - The shell's F2 to F5 are `app.hdr`, `app.hdr.peak`, `app.hdr.false_colours` and
    `app.calibrate`; the overlay's 1 to 9 are `app.profile.group N`.
- **`bind`, `unbind` and `binds`:**
  - a bound line may hold several commands, split at `;`;
  - its output shows for three seconds at the bottom left;
  - `forge.cfg`'s `bind` lines run once the demo is ready, before `--exec`;
  - keys are named by what they type in the layout (winit's `key_without_modifiers`), or by
    their name;
  - the shell's keys refuse a bind;
  - `cycle NAME VALUE VALUE...` steps a setting through the values listed, so one key goes
    through `dlaa`, `taa` and `off` without `toggle`'s `dlss` (the owner's ask, the same day).
- **Checks:**
  - unit tests of the binds, their key names and the file's console lines;
  - a run with a test `forge.cfg`, the console listing its binds and the new settings;
  - `docs/demos/console.md` maps each old key to its setting, with a `forge.cfg` that puts them
    back on the function keys.
- **Mouse buttons and the game's mouse look** (2026-10-19, asked by an out-of-tree town for a
  rifle and its scope):
  - `Input::button_down`, `button_pressed` and `button_released` give each mouse button held,
    pressed and released this frame.
  - A button binds as a key does: `bind mouse1 …`, with mouse1 the left, mouse2 the right,
    mouse3 the middle, mouse4 and mouse5 the sides.
  - `app.mouse_look`, a setting, captures and hides the cursor (locked where the platform can,
    else kept in the window). The view then turns with no button held, and the right button is
    the game's, for aiming.
  - Esc first lets the mouse go, and a second Esc quits.
    - **Since 2026-10-22 (the owner's choice):** as games do before their menu, which Forge has not yet. The mouse look stays on, a click in the window takes the cursor back (the game gets no press), and the game runs on meanwhile, as a networked one must. It used to turn the mouse look off, to be toggled back.
    - **Since 2026-10-23 (#352, the owner's ask):** Esc opens the game's menu in every demo (resume, settings, quit, `forge_app::menu`), the mouse look's cursor let go while it shows and taken back on resume. A solo game holds behind it (`Context::paused`, a zero `dt`); a networked one (`Demo::networked`) runs on. Quitting moved into the menu: Esc no longer closes the window, but while loading.
  - Losing the window's focus lets every key and button go and frees and shows the cursor (since
    2026-10-22: it stayed held and hidden, and Alt+Tab fought it); coming back, the mouse look
    takes the cursor again, unless Esc let it go.
  - The fly camera's right-button look is unchanged when the mode is off.

## D-070 — DLSS is the reference image, FSR replaces TAA everywhere else ✅ (2026-10-18, #286, the owner's decision)

The owner, 2026-10-18: "DLSS 5 is coming, I would prefer we work with the best possible. With FSR
as well. Running tests once in a while when drivers change seems less tedious than testing DLAA
all the time." Until now TAA was the default of every scripted run and the fallback (D-024).
The captures and their checks ran under TAA, while the owner looked at the demos under DLAA
(D-045). Two paths were tested and judged, and time went into TAA's own image: its shimmer, its
sharpening, and the lamp filter's flicker under TAA. The last two run-to-run flakes (#277 and
#281) were TAA's passes compiling two ways.

**Decided:**
- **DLSS is the image on NVIDIA,** in interactive runs and in the captures:
  - every demo draws with DLAA by default, scripted runs included;
  - the capture batch (`tools/captures.sh`), the map check and `tools/verify.sh` capture under
    DLAA, and the looks and the shimmer are judged under it alone.
- **A driver or DLSS change is a re-baseline, not a failure:**
  - each accepted set's manifest records the driver's version and the DLSS DLL's;
  - when either differs, `tools/verify.sh` asks for a baseline at the base commit first.
  - The DLL ships with the build, from the Streamline SDK the repository pins. It changes when
    Forge upgrades it. The NVIDIA app's overrides stay off in test runs, and the preset is set.
- **FSR is the path off NVIDIA:**
  - FSR 3.1 first: AMD's FidelityFX, MIT, with a Vulkan backend, on every vendor, so it is tested
    on the RTX 5070 Ti;
  - FSR 4 (AMD's ML upscaler, RDNA 4) when there is a card to test it on and a Vulkan path.
- **FSR replaces TAA.** FSR 3.1 is open shader code that runs on any vendor's GPU, older ones
  included, so it takes over everything TAA did:
  - the anti-aliasing at native size, as DLAA does;
  - the sharpening (RCAS, which D-045 added after TAA);
  - the averaging of the noisy effects over frames: the soft shadows, the lamps' penumbrae,
    GTAO.
  - The owner first kept TAA for older hardware, then dropped it (2026-10-18): "FSR is MIT and
    open source, it can replace TAA everywhere."
  - TAA's resolve and sharpening passes go once FSR 3.1 is in and checked. Until then TAA is
    frozen: no more work on its image and no more checks of it.
  - The jitter, the motion vectors, the reactive mask and the exposure stay, since DLSS and FSR
    read them.
- **Kept as tools:**
  - an "AA off" run, to tell a change in Forge's shading from one in DLSS's reconstruction;
  - SSAA (2 × 2), the yardstick of the shimmer measurements.
- **DLSS 5** comes through Streamline like DLSS 4, as an upgrade of the SDK, when NVIDIA publishes
  it.

**Determinism (checked before the switch, 2026-10-18, at 4c6efbf6, driver 617.42):**
- **What ran:** eight scenes, one or more from every demo, each under DLAA (`temporal/DLSS` in every
  log, 0.46–0.52 ms), twice on the mesh path and once on the fallback:
  - the city;
  - the island's water;
  - the lab's smoke yard and its drop at frame 600;
  - the village's hall by day and its street at 22:00;
  - the ballad at frame 600;
  - the planet's ground.
- **The result:** every capture's bytes were equal, run to run and mesh against fallback, so the
  checks keep their 0 px.
- **D-045's "a code or two from run to run" (2026-10-03) was Forge's,** not DLSS's. The shading
  and TAA compiled two ways until #277 and #281.

**What DLSS costs a run (2026-10-18):**
- **Where the time went:** under DLAA, Tier 1 took 50 minutes instead of about 18. Every capture
  is its own run, and DLSS added 3.0 s to each one (lab 1.4 → 4.3 s, city 2.4 → 5.4 s, planet
  4.3 → 7.4 s). Streamline's own log shows two causes:
  - when it starts, it checks the signature of every plugin in its folder and asks NGX for each
    one's requirements; the first request alone took 1.0 s;
  - when the device is created, NGX finds every `nvngx_*.dll` in that folder (DLSS, frame
    generation, ray reconstruction, DeepDVC) and starts each one, 1.1 s for the lot.
- **Fixed:** Streamline now loads from a folder holding only what DLSS needs. Forge makes it
  itself beside the SDK's folder (`streamline-sdk/bin/forge-dlss/<key>`), from hard links to
  six of the SDK's files.
  - DLSS now costs 1.4 s a run (lab 2.7 s, city 3.8 s, planet 5.7 s).
  - The images are the same to the byte.
- **Left:**
  - Streamline's first request to NGX, 0.46 s;
  - NGX loading DLSS at device creation, 0.56 s.
  Both happen in every process, so the remaining cost is per run.
- **Tier 1 went from 50 minutes to 25** (5c74499c). The full batch then moved to the night
  (D-071).

**FSR 3.1: the patents, and the owner's choice (2026-10-18):**
- **The version:** FSR 3.1.4, from AMD's FidelityFX SDK v1.1.4 (MIT), in the ignored
  `fidelityfx-sdk/`. It is the last SDK with a Vulkan backend: the FSR SDK 2.x says Vulkan is not
  supported.
- **The patent check found two families that read on it:**
  - **AMD:** US 12,293,485 B2 and US 12,493,990 B2, granted in 2025 and live until about 2043.
    Their EP applications are pending, and the JP patents are granted. Their claim 1 covers the
    thin-feature "locks", which FSR 3.1.4 still creates (`prepare_reactivity`) and uses
    (`accumulate`).
  - **NVIDIA:** US 10,116,916 B2 and US 10,362,289 B2, US only, live until about 2037. They
    cover clipping the history to a box made from the neighbourhood's mean and standard
    deviation, which FSR does. So does Forge's own TAA, until FSR replaces it.
- **The licences:** the MIT licence never says "patent", and AMD has no pledge. French law
  (CPI L613-8) wants a patent licence in writing. Godot ships FSR 2.2 as native AA.
- **What was clear:** RCAS, the masks and luma instability have no patent of their own. Sony's
  foveated temporal supersampling patents are still open: their granted claims were not read.
- **The owner's choice:** AMD's shaders as published, locks included: AMD's own code is the
  closest Forge can stay to its grant. Only the history's box changes, to a min/max
  neighbourhood clamp (Karis 2014, older than NVIDIA's priority date). The swap is a small
  patch of AMD's header, kept in Forge, and its effect on ghosting and shimmer is measured.

**FSR 3.1 built (2026-10-18), `--set r.aa=fsr` in every demo** (`forge_render::FsrUpscaler`):
- **AMD's headers are kept in Forge,** in `shaders/third-party/fidelityfx/`. Their licence is MIT,
  and FSR must be there on any machine, unlike RTXDI's SDK.
  - Forge's bridge (`shaders/fsr_bridge.slang`) gives them their callbacks over the bindless images
    and a frame block, with AMD's binding numbers as slots in each pass's push constants.
  - One Slang file a pass: Slang compiles them as Slang, not HLSL, so AMD's loops build unchanged.
  - The options are fixed: 32-bit floats, HDR input, motion at the render's size without jitter,
    reversed infinite depth, and the downsampler without wave operations (no subgroup size
    assumed).
- **Its host code is in Rust:** the images, the constants each frame, and the eight passes in
  the render graph (`temporal/FSR …`), plus a clear pass for what AMD clears. The motion and the
  jitter are Forge's TAA's, the same conventions (motion in UV, scale 1).
- **Found on the way:** AMD's new-locks image is read everywhere and written only on thin
  features; its accumulation zeroes it for the next frame. As an aliased transient it read
  garbage, so the island's water differed 15.7 % between the mesh path and the fallback. It is now
  kept between frames, and every image kept is cleared on the first frame (Vulkan's memory is
  not zero).
- **Determinism:** run to run and mesh against fallback are 0 px. That was checked in the city,
  the ballad at frame 600, the planet's ground, the lab's drop and smoke yard, and the island's
  water. Its passes compile with `-fp-mode precise`, as TAA's do.
- **The measures,** in the city at 1600 × 900:
  - the still camera: the pixels changing between frames 10 apart;
  - the turn: 3° a second, the pixels whose change from frame to frame differs from SSAA 2 × 2's;
  - the error against SSAA at frame 70.

  | | Still | Turn | ꟻLIP vs SSAA | Frame (GPU) |
  |---|---|---|---|---|
  | FSR, min/max box (Forge's) | 0.074 % | 34.0 % | 0.0414 | 3.75 ms |
  | FSR, AMD's variance box | 0.487 % | 33.3 % | 0.0408 | 3.58 ms |
  | DLAA | 0.249 % | 36.9 % | 0.0434 | 4.08 ms |
  | TAA | 4.98 % | 40.6 % | 0.0457 | 3.50 ms |

  - The min/max box steadies the still image six times over. It loses 0.7 points in the turn and
    1.5 % of ꟻLIP.
  - FSR beats DLAA here on all three measures.
  - Its passes take 0.31–0.35 ms, against DLSS's 0.63 ms.
- **The captures:** `ast-fsr600`, `city60-fsr` (a sentinel) and `lab-drop90-fsr` on both paths,
  plus a validation run of the city under FSR.

**FSR where DLSS is not (2026-10-18):**
- FSR is the default anti-aliasing of the city's galleries, the planet and the ballad wherever
  DLSS does not run: an AMD or Intel GPU, or a build without Streamline. On NVIDIA nothing
  changes.
- `r.aa dlaa` at the console on such a machine shows FSR.
- **The ballad on this machine's AMD iGPU** (RDNA 2, frame 600, `--fixed-step`):
  - FSR came on by itself, with no error or device loss in 600 frames.
  - It is 0.27 % of pixels and ꟻLIP mean 0.0057 from the 5070 Ti's FSR image.
  - FSR's passes take 18.4 ms of the iGPU's 128 ms frame, the accumulation 8.3 ms of them.

**TAA's resolve and sharpening removed (2026-10-18, the owner's go):**
- **What went:**
  - the resolve (`resolve_history_main`), its Lanczos-3 history and the pass that showed it
    sharpened by RCAS (`sharpen_main`);
  - their settings and flags: `r.aa taa`, `r.taa.sharpen`, `r.taa.sharpen_stops`, `--rcas`,
    `--no-rcas`, `--taa-catmull-rom`, the ballad's `--taa-blend` and `--upscaler taa`;
  - `FORGE_HASH_SHARPEN`.
- **What stays:** `Taa` keeps the jitter, the reprojection and the motion vectors pass, and
  `TaaFrame` is what DLSS and FSR take. The reactive mask and the exposure go to FSR as before.
- **The anti-aliasing is DLAA, FSR or off.**
  - FSR is made at the start wherever DLAA does not run, or the first time the console asks for
    it (`r.aa fsr` on NVIDIA).
  - With it off, nothing is jittered, and the display pass shows the frame as drawn.
  - One display pass per demo now serves all three. That fixed two slips of the FSR commit:
    FSR's image missed the night's Purkinje shift and F2's HDR switch.
- **The tank's water** bends a scene anti-aliased by an FSR of its own (#156), in place of the
  second TAA:
  - its passes are "liquid/scene FSR …", 0.30 ms at 1600 × 900 against TAA's 0.079;
  - `--no-behind-aa` (was `--no-behind-taa`) shows the scene aliased.
- **Images:** DLAA's and FSR's are unchanged (the city under each: 0 px). Two kinds changed:
  - with the anti-aliasing off (the ballad's `--no-taa` captures), TAA's resolve still blurred
    each pixel with its neighbours' 1 % weights, and the frame is now shown as drawn: frame 240,
    0.79 % of pixels, ꟻLIP mean 0.0042;
  - the tank's scene through FSR rather than TAA: `lab-tank90`, 0.76 %, ꟻLIP mean 0.0035.

**FSR's upscaling modes (2026-10-18):**
- **The modes** are AMD's: native (its anti-aliasing), quality, balanced, performance and ultra
  performance, drawn 1, 1.5, 1.7, 2 and 3 times smaller (`FsrMode`).
- **Where:** in the ballad, the demo that upscales.
  - `--upscaler fsr-quality` and the others choose one; `r.aa fsr` with `r.fsr.mode` at the
    console.
  - A DLSS mode asked for where DLSS cannot run gives FSR's mode of the same name.
  - U cycles DLAA, DLSS's four modes, then FSR's five (FSR's alone without DLSS).
- **The host code** already followed AMD's for any size (the render size, the jitter sequence's
  length, the constants), so the modes needed no change to FSR itself.
- **The textures' mip bias,** which both upscalers ask for when they draw smaller:
  - `MeshletRenderer::texture_lod_bias`, log2(render / output), scales the UV derivatives in
    the resolve, in the frame block's spare word;
  - at the output's size it is ×1, so no other image changes;
  - AMD's extra −1 stop is left out: it would sharpen the textures at native size too and move
    DLAA's reference image;
  - in the ballad it brings each mode about 1 % closer to DLAA (ꟻLIP 0.0442 against 0.0444 at
    FSR Quality, 0.0312 against 0.0317 at DLSS Quality); `--no-texture-bias` is the A/B.
- **The ballad's frame 600 at 1600 × 900,** against DLAA. The ꟻLIP is against DLAA's own image,
  which favours DLSS's modes.

  | Mode | Drawn at | GPU frame | The upscaler's passes | ꟻLIP vs DLAA |
  |---|---|---|---|---|
  | DLAA | 1600 × 900 | 2.22 ms | 0.55 ms | — |
  | FSR (native) | 1600 × 900 | 1.96 ms | 0.37 ms | 0.023 |
  | FSR Quality | 1066 × 600 | 1.40 ms | 0.25 ms | 0.044 |
  | DLSS Quality | 1067 × 600 | 1.79 ms | 0.51 ms | 0.031 |
  | FSR Balanced | 941 × 529 | 1.39 ms | 0.23 ms | 0.052 |
  | DLSS Balanced | 928 × 522 | 1.70 ms | 0.52 ms | 0.038 |
  | FSR Performance | 800 × 450 | 1.29 ms | 0.23 ms | 0.061 |
  | DLSS Performance | 800 × 450 | 1.62 ms | 0.52 ms | 0.049 |
  | FSR Ultra Performance | 533 × 300 | 1.16 ms | 0.21 ms | 0.085 |
  | DLSS Ultra Performance | 533 × 300 | 1.46 ms | 0.52 ms | 0.074 |

  - FSR Quality draws its image 0.4 ms faster than DLSS Quality here, and DLSS's passes cost
    about the same at every size.
  - FSR Quality's mesh path and fallback are 0 px apart. `ast-fsrq600` captures it on both
    paths.

**RCAS after FSR, on by default at 0.5 (2026-10-18, the owner's ask):**
- **The settings:** `r.fsr.sharpen` and `r.fsr.sharpness` (AMD's scale: 0 is two stops below
  RCAS's strongest, 1 its strongest), in the galleries, the planet and the ballad. AMD's RCAS
  pass runs after the accumulation, 0.02 ms at 1600 × 900. The tank's own FSR stays unsharpened.
- **Where it sharpens:** RCAS's limiter assumes colours in [0, 1], as FSR 1 runs it on the
  display's signal. FSR 3.1 hands it the history, pre-exposed HDR. Three ways were measured:
  - **as AMD ships it, on linear HDR:** the room's edges barely moved (MTF50 still 0.52, panning
    0.33 → 0.34), but the city shimmered up to six times more. Bright edges over 1 were left
    alone while the darker detail was sharpened;
  - **through AMD's reversible tone map, x / (1 + its largest channel):** panning 0.33 → 0.36 at
    0.75, still unchanged;
  - **through that tone map and a square root (gamma 2, Forge's bridge):** the even sharpening
    RCAS is meant for, kept. The RCAS pass reads the history through it and writes back through
    the inverse (`FSR_RCAS_TONEMAPPED` in `shaders/fsr_bridge.slang`). AMD's headers are
    unchanged.
- **The measures:**
  - the sharpness room's MTF50 (the board's two edges and the wall square's), still and
    panning at 2 m/s;
  - the city's still shimmer (pixels changing between frames 10 apart);
  - the city's turn at 3° a second against SSAA 2 × 2.

  | | Still MTF50 | Panning MTF50 | City still | City turn |
  |---|---|---|---|---|
  | An ideal pixel | 0.60 | 0.60 | | |
  | DLAA | 0.59–0.61 | 0.43–0.47 | 0.249 % | 36.9 % |
  | FSR unsharpened | 0.51–0.52 | 0.32–0.33 | 0.074 % | 34.0 % |
  | FSR, RCAS 0.25 | 0.55–0.57 | 0.35–0.36 | 0.140 % | 35.1 % |
  | **FSR, RCAS 0.5** | **0.57–0.58** | **0.36–0.38** | **0.195 %** | **36.3 %** |
  | FSR, RCAS 0.75 | 0.58–0.60 | 0.38–0.40 | 0.340 % | 39.2 % |
  | FSR, RCAS 1 | 0.61–0.62 | 0.41–0.43 | 0.946 % | 47.3 % |

- **The default, 0.5:** a still edge nearly an ideal pixel's, and panning as sharp as TAA's
  RCAS was. The city still shimmers less than under DLAA on both measures. From 0.75 it shimmers
  more than DLAA.
- **The captures:** FSR's four, on both paths (`city60-fsr`, `lab-drop90-fsr`, `ast-fsr600`,
  `ast-fsrq600`), changed as meant (ꟻLIP mean 0.013–0.019). Mesh against fallback stays 0 px.
- **Patents:** none found that reads on RCAS (`docs/research/fsr-3-1.md`: US 12,020,408 B2
  needs a blur circuit before the sharpener).
- **On this machine's AMD iGPU** (RDNA 2, the ballad's frame 600, the default FSR):
  - no error or device loss in 600 frames;
  - `temporal/FSR sharpen` takes 1.31 ms, FSR 19.4 ms in all of a 135 ms frame;
  - the sharpening changes the image as on the 5070 Ti: 7.47 % of pixels, ꟻLIP mean 0.0144,
    against 7.45 % and 0.0144;
  - AMD's image is 0.36 % of pixels from the 5070 Ti's (ꟻLIP 0.0060), against 0.27 % (0.0057)
    unsharpened.
- **The city on the same iGPU** (`city60-fsr`'s view, frame 60, the default FSR with RCAS):
  - no error in 60 frames;
  - a 305 ms frame, of which the probes' rays take 115 ms and FSR 18.8 ms (the sharpening
    1.31);
  - the image looks the same as the 5070 Ti's. It is 5.1 % of pixels and ꟻLIP 0.0149 from it,
    on the windows' edges, the rocks and the clouds' rims: texture filtering and ray-traced
    light, which differ between vendors, not FSR's passes (an AA-off pair would confirm it).

**Amended 2026-11-08 (the owner: "a trail from the bottles when the camera is moving"):** once
the exposure has settled, DLSS's pre-exposure is at most 1 (`STEADY_STOPS`, −4 to 0 stops from
EV 15; the ceiling was +4).
- **Why:** the private district's bar is exposed at about EV 2.8, past that ceiling, so DLSS got
  the frame 16 times dimmer than it is shown. It then kept what a near object had just
  uncovered: echoes of a bottle's neck, one a frame behind the other, on the dark room behind
  it. At 1 they are gone; FSR shows none either once the bottle's motion is right. The bottle's
  own tearing was D-050's virtual point (that entry's amendment of the same day).
- **Still camera, DLSS Quality, old against new** (pixels changing by more than 30 codes between
  two frames of 3.69 M): the bar 87 → 54, a street 7,054 → 7,027; by more than 2 codes, 14.62 →
  14.68 % and 3.23 → 3.24 %. No new flicker.
- **What stays, and its limits:** an adapting exposure is still followed, then eased back at
  0.004 stops a frame, and a scene under EV 15 now relies on that for every change (before,
  under EV 11).
  - **A fall of more than 6 stops** runs out of `FOLLOW_RANGE`, which starts at 0 now and had 10
    stops of room from the old 4: from a dark room straight into full day, DLSS does not see the
    rest of the fall, the fault of 2026-10-24 (a cross-hatch for a second or two). The private
    district's exposures span 6.4 stops, so 0.4 of it there.
  - **Coming into the dark**, the pre-exposure rises above 1 with the adaptation (as far as 2⁸)
    and eases back: 1,500 frames for 6 stops, 25 s at 60 fps. Until it is back near 1, the
    echoes can show again, as they always did there.
- **Images:** every DLSS capture exposed under EV 15 moves a little, since DLSS sees a brighter
  input.

**Next:** FSR 4, once there is an RDNA 4 card to test on and its SDK has a Vulkan path.

## D-071 — The day check and the night's full batch ✅ (2026-10-18, #286, the owner's choice)

Under DLAA (D-070) a Tier 1 took 50 minutes, and 25 after the Streamline fix. Every push of a
shared change (`forge-gpu`, the render graph, a shared shader, `Cargo.lock`, the scripts) waited
for it. The owner asked whether the tiers could run at night. Of three ways (nights with a short
day check; nights plus a one-process capture mode; Tier 1 before every push, made faster), they
chose the first: "Nights + short day check".

**Decided:**
- **By day,** a shared change gets the day check (`tools/verify.sh` picks it, Tier 0):
  - the gate;
  - the sentinels, now one or more captures of every demo (D-043's ten, and since D-071 the
    island's `shot-island`, the lab's `lab-materials60` and the planet's `planet-ground`);
  - the sets the mapped paths select (until the same day: see below);
  - all on both paths, with the sentinels' validation.
  It takes a few minutes, and the change is pushed when it passes.
- **Trimmed the same day (the owner):**
  - **What happened:** with a demo's sets on both paths, the day check took 24 minutes (FSR's
    commits, D-070). The owner: "we have to reduce the number of unnecessary tests".
  - **The day check is now the sentinels alone,** on both paths with their validation. The sets
    the mapped paths select wait for the night's batch.
  - **A change whose images the session has already compared** with targeted captures against
    the accepted set is pushed after the gate (`--tier gate`).
- **At night,** a Windows scheduled task ("Forge night check", 00:30, run when missed; 01:30 from 2026-10-18, the owner) starts
  `tools/night.sh`, which:
  - checks out `origin/main` in its own worktree (`../forge-night`), whose ignored SDK and
    asset folders are junctions to the main checkout's;
  - runs Tier 1 on what was pushed, or Tier 2 when a milestone is due;
  - keeps the accepted sets in the main checkout's `captures/accepted/`.
  `captures/night/latest.txt` holds the verdict.
- **The morning's first session** reads `latest.txt`. A failed night is fixed before new work:
  - find the commit with `--base`;
  - fix it, or, when images moved on purpose, rerun `--tier 1 --expect ...`.
- **Unchanged:** mapped changes keep their Tier 0, and the map check (D-062) runs by day after a
  change to the village. `--tier 1` and `--tier 2` still run by hand, before a showcase.

**The cost:** a regression that only the full batch sees reaches `main` for up to a day before
the night finds it. The night narrows it to the day's commits.

**Not done, kept for later:** one process making many captures, the scenes changed inside it
(the lab first, 72 of 108 captures a path). It would save about 2 s a capture, but every reset
must leave nothing behind (DLSS's history, probes, exposure, counters, pools), proved by 0 px
against fresh runs.

## D-072 — Scripting in Luau through `mlua`, outside the predicted step ✅ (proposed 2026-10-18 on the owner's question; Luau the owner's choice, the rest the owner's go the same day)

The owner asked on 2026-10-18 what the industry uses for scripting and which language Forge
should take. The research is `docs/research/scripting.md`. Asked which language to use without the
engine's determinism constraints, Claude recommended Luau, and the owner asked for this entry.
D-035 placed "the scripting and mod-code decision" at Phase 10. This entry takes the language now
so that missions and mods are designed for it. Nothing is built until a mission needs code.

**Chosen (the owner):** Luau, Roblox's typed descendant of Lua 5.1 (MIT), embedded through the
`mlua` crate (MIT) in a new crate, `forge-script`.

**Decided (the owner's yes to the four questions below, missions as data first):**
1. **Missions as data first.** A `Mission` record in `forge-data` packages:
   - objectives;
   - triggers: a volume entered, an item picked up, an entity killed, a timer, all-of, any-of;
   - steps combined in sequence, all together (`sync`), first wins (`race`) or in the background
     (`branch`), as Verse's structured concurrency names them.

   A Rust system evaluates it at the fixed tick, so it is digested, saved by stage and
   hot-reloaded. The generator writes these records for generated maps (D-062). Luau is for
   what the records cannot say.
2. **Where scripts run:** on the server and in the client's interface. They never run inside the
   predicted step (D-010, D-016).
   - A script reads the world through a query API by stable ids.
   - Its effects are commands stamped with a tick. They travel in the event stream as D-034's
     director commands do, and Rust systems on every machine apply them.
3. **State lives in records** (as Factorio's `storage`): saves, snapshots and hot reload then
   work, and the VM's globals are caches. Coroutines are used only for what may restart from a
   checkpoint, such as cinematics.
4. **The sandbox:**
   - `Lua::sandbox` (read-only libraries and globals, an environment per script), with Forge's
     libraries only;
   - a memory limit;
   - a per-tick instruction budget through Luau's interrupt;
   - one VM per mod or per trust tier;
   - no loading of untrusted bytecode.
5. **Reproducible for replays and captures:**
   - `math.randomseed` from `Seed::derive`;
   - `math` mapped to `dmath`;
   - no clocks;
   - ordered-iteration helpers, since tables keyed by objects hash by address;
   - no weak tables in simulation scripts.
6. **Tooling:**
   - definition files generated from the Rust API for `luau-lsp`;
   - `luau-lsp analyze` in the gate;
   - an F1 zone per script;
   - a changed module reloaded while the game runs.
7. **Mods:** data packages first (D-035). Then Luau mods, server-first, with the client running
   only interface and presentation scripts.
8. **WebAssembly stays D-035's route** for mod code that must enter the predicted simulation or
   comes in another language. Before it is built, the claims of Zynga's US 8,449,397 B2 family
   (scripts run independently on client and server, to about 2031) are read.
9. **No visual scripting for now.** A graph view of mission records can come with the editor.

**Why Luau:**
- **Its sandbox is built in,** where plain Lua needs one built by hand.
- **Gradual types:** a script starts untyped and gains types. `luau-lsp` gives completion and
  checks in an editor and in CI.
- **A fast interpreter without a JIT:** consoles and iOS forbid JIT, and native code is opt-in
  per module.
- **Lua's simplicity,** readable by the modders of Factorio, Project Zomboid and WoW.
- **Maintained and adopted:** Roblox maintains it under MIT. Remedy replaced its own language with it
  for Alan Wake 2, and Warframe uses it.
- **Live editing,** Remedy's reason for the switch.

**Not chosen:**
- **Lua 5.4/5.5:** the same culture and 64-bit integers, but the sandbox built by hand and types
  only through annotations.
- **TypeScript on QuickJS:** the largest talent pool, but a garbage collector and an engine that
  changes fast.
- **WebAssembly only:** the best sandbox and deterministic, but modders and designers must compile.
- **Rhai, Rune:** easy to embed in Rust, but slow and with small communities.
- **C# on CoreCLR:** no in-process sandbox exists in modern .NET.
- **Verse:** promised as open source in 2022 and not released.
- **Angelscript:** a C++ library whose strength is Unreal's integration.

**What it costs:**
- **No integer type:** ids travel as opaque handles.
- **A C++ build inside `mlua`:** Forge's `unsafe` rule holds, since `forge-script` uses only the
  crate's safe API.
- **Credits:** Luau and `mlua` get their lines in `CREDITS.md`, and `cargo run -p credits` updates
  the crate list.
- **No standard debugger outside Roblox Studio:** logging and the F1 zones come first.
- **Bindings and definition files to maintain.**

**It amends D-035:**
- D-035's "Mod code, if wanted: Wasm components" becomes Luau for mod code outside the
  prediction, and WebAssembly for mod code inside it.
- D-035's "Not chosen: Lua in the deterministic simulation" stands: scripts stay out of it.

**Patents:**
- **Nothing found** that points 1–7 build on.
- **Zynga's family** is read before point 8.
- **Visual-scripting editors to avoid:** one that draws a model's mesh in the graph (Amazon,
  US 10,909,754 B1) and one of stacked code tiles (Epic, US 11,144,284 B1).

**The demo that would prove it:**
- **The same objective twice:** the village map's objective (reach a room, find an item), written
  once as a mission record and once as a Luau script that sends the same commands.
- **What must match:** both give the same digests and pixel-identical captures.
- **The budget:** a script that loops forever is stopped by its budget and reported, with its F1
  zone.
- **Hot reload:** a script edited while the map runs takes effect without losing the mission's
  stage.
- **The numbers:** a script's time per tick, its memory, and its reload time.

**Decided (the owner, 2026-10-18): yes to all four, missions as data first.**
1. Missions are data first (point 1). The first mission record is the next step (#287).
2. Scripts never run inside the predicted step (point 2).
3. Mods are server-first; the client runs only interface and presentation scripts (point 7).
4. No visual scripting until the editor (point 9).

**The first host: `forge-script`, sandboxed Luau for data (#292, 2026-10-18).** A scene built
outside the repository asked to read third-party Lua 5.1 files offline: pure data tables, and
rules that build their tables in loops. The owner approved `mlua`'s download.
- **The crate:** `mlua` 0.12 with Luau, built from its sources (`luau0-src`, Luau 0.740) by
  the MSVC toolchain Jolt already needs. No `unsafe` in Forge's code.
- **The sandbox** (`Script::new(Limits)`):
  - Luau's sandbox mode: the standard libraries and their metatables are read-only, and a
    chunk's globals go to an environment of their own;
  - libraries loaded: coroutine, table, string, utf8, bit32, math, buffer. There is no `io`, `os`,
    `debug` or `package`;
  - removed: `load`, `loadstring`, `dofile`, `loadfile`, `require`, `getfenv`, `setfenv`,
    `collectgarbage`, `newproxy`;
  - `load` and `loadstring` come back only with `Limits::allow_load`, compiling into the same
    sandbox;
  - `require` reads only from `Limits::require_root`: `a.b` or `a/b` as `a/b.luau` or
    `a/b.lua`, a name that would leave the folder refused, each module loaded once.
- **The caps:**
  - a budget of Luau's interrupts, which come at calls and loops' back edges: a measure of work,
    the same from run to run;
  - a wall-clock deadline, read every 1 024 interrupts;
  - a memory cap on the whole state.

  Over any of them the chunk ends with an error (`Error::Budget`, `Error::Memory`), and the
  state runs the next chunk.
- **Stubs:** `Script::set` (a global from `Data`) and `Script::function` (a Rust function of
  `Data` arguments returning `Data` or an error message), so code written against another
  engine's interface can partly run. The globals a caller set are left out of
  `Script::globals`.
- **Results as Rust values** (`Data`): nil, booleans, integers (numbers with no fraction),
  numbers, strings, lists (tables keyed exactly 1 to n), maps (by integer, string or other key),
  and opaque values by type name. A table that holds itself, or nests deeper than 64, is an
  error, not a loop.
- **Errors name the chunk and the line** (`rules.lua:2: …`), as syntax or runtime errors.
- **Lua 5.1 in Luau.** Closures, varargs and `select("#")`, metatables and `__add`, string
  patterns, `pcall` and `error`, `unpack`, `string.format` and `table.concat` run as written.
  Missing: `setfenv`, `getfenv`, `module`, `table.getn`, and the loaders above. The corpus will
  show which matter.
- **Checked:** 13 unit tests on a synthetic corpus (nothing from any game):
  - a returned table and globals built in a loop, both as data;
  - a 1 MB data table (12 264 records) read in 93 ms, the state holding 8 MB;
  - the sandbox's missing names, and the read-only libraries;
  - an endless loop stopped by the budget and by the deadline, the state running the next
    chunk;
  - a memory bomb stopped at a 16 MB cap;
  - endless recursion an error, not a crash;
  - errors with their chunk and line;
  - stubs from Rust;
  - `require` kept to its folder (`..`, absolute paths and drive letters refused);
  - `load` only when allowed;
  - Lua 5.1 idioms;
  - a table that holds itself refused.
- **Not yet:** scripts in the game's tick, mods, missions calling Luau, the F1 zone, hot reload
  of scripts. D-072's demo, the same objective as a record and as a script, comes with those.
- **A first corpus** (2026-10-19, run by a session outside the repository on 29 files of Lua 5.1
  data tables and rule scripts, with 50 one-feature probes against PUC Lua 5.1.5):
  - **What it found:** all 29 parsed, and 16 ran as they were. All 13 failures came from
    `require`:
    - cycles (a module requiring its parent, which requires it) ended in "error in error
      handling", with no chunk or line;
    - names taken from a folder's listing kept their `.lua`.
  - **The probes:**
    - the removed names failed with "attempt to call a nil value", which names nothing;
    - `math.mod` and `string.gfind` were missing;
    - `_G` was the frozen base table;
    - library writes were refused.
  - **What changed, the same day:**
    - `require` marks a module loaded before running it, so a cycle gets `true`;
    - it drops a trailing `.lua` or `.luau`, and `a/b` and `a.b` are one module;
    - the removed names raise "… is not available in this sandbox";
    - `collectgarbage("count")` and `gcinfo` answer;
    - `math.mod` and `string.gfind` are shims;
    - `_G` is the chunks' environment;
    - `Limits::lua51` gives writable libraries and one table of globals.
  - **Left as Luau has it:**
    - numbers written in their shortest exact form rather than 5.1's `%.14g`. That changes
      `tostring` and `..`; the corpus had no table key built that way;
    - no `arg` table in vararg functions.
  - **Speed:** a 1.1 MB table ran in 25 ms, and the whole corpus in 1.2 s.
  - **Tests:** 16.
- **Game scripts, built 2026-10-19** (#319, `forge_script::Game`, asked by an out-of-tree test
  scene). D-072's second point, the first step:
  - **Events:** `game.on(name, handler)`. The host sends events (`Game::send`) and ticks
    (`Game::tick`, the `"tick"` event with the tick's number). Each event's handlers run in the
    order registered.
  - **Timers:** `game.after(seconds, f)`, `game.every(seconds, f)` and `game.cancel(n)` count the
    host's ticks, never a clock. Timers due on one tick run in the order set, before its
    handlers. A repeating timer is set again before its handler runs, so the handler can
    cancel it.
  - **Entities:** handles (`Entity`, userdata with `:id()`, equality and `tostring`), never
    plain numbers. They come from events and from `game.find(kind)`.
  - **Reading the world:** `game.get(entity, field)` and `game.find(kind)`, through the host's
    `World` trait, answer only while an event runs. The world is lent through Luau's registry:
    Luau resolves `game.find` once, when a handler loads (an import of its safe environment),
    so swapping the table's field was never seen.
  - **Effects:** `game.command(name, ...)` queues commands stamped with the tick
    (`Game::commands`), for Rust systems to apply on every machine.
  - **Caps:** each event runs under the script's caps, its budget of interrupts reset at each.
    A handler over its budget ends with an error, and the game goes on.
  - **Tests:** 4 (events in order, timers, entity handles through a world, a handler over its
    budget).
  - **Definitions:** `crates/forge-script/defs/game.d.luau` describes `game` and `Entity` for
    `luau-lsp`, written by hand. A test checks that it names every function of the table.
    luau-lsp 1.70 reads classes declared as `declare extern type … with`.
  - **`luau-lsp analyze` in the gate** (the owner's go for the download, 2026-10-19):
    - `tools/luau-check.sh` checks every `crates/forge-script/scripts/*.luau` against the
      definitions, with `--platform=standard`.
    - CI downloads release 1.70.1 on Linux and checks its SHA-256 before running it.
    - Locally the release is unpacked in `luau-lsp/` (ignored by git); without it, the check
      says so and passes.
    - It caught a misspelt `game.fnd` and a string passed as a timer's seconds.
  - **The example:** `scripts/pursuit.luau` sends the healthy zombies after a noise and makes
    the hurt ones flee. A test runs it, and the check type-checks it.
  - **Left:** generating the definitions from the Rust API; an F1 zone per script; hot reload.
- **Then (2026-10-23, #319):**
  - **Hot reload:** `Game::reload(source)` starts the script again at the tick it is at. The new
    script's handlers and timers replace the old ones (its timers count from that tick); the
    commands not yet taken stay. A source that fails to load leaves the old script running and
    returns the error. State a script keeps in its own locals starts afresh: D-072's third point
    (state lives in records) is what makes a reload lose nothing.
  - **Input as a built-in event:** `Game::input(action, value)` sends `"input"` with the action's
    name (the game's, as its `bind` lines name it, D-069) and its value (1 pressed, 0 released,
    −1 to 1 for an axis).
  - **Time per event:** `Game::timings()` gives each event's runs and milliseconds since the last
    call (`"timers"` for the timers), for the host's profile. The engine's F1 overlay takes them
    once a demo runs a game script; none does yet.
  - **Tests:** 2 more (a reload at the same tick, a failed one; input and the timings).

## D-073 — Missions as data: the record, its evaluator and its two saves ✅ (2026-10-18, #287, D-072's first point)

D-072 put missions as data first. This is the first version, built in a crate of its own,
`forge-mission`, and proved on the physics lab's playground (`--lab walk --mission`), whose
world is the one fully digested simulation (`forge_sim::Simulation`). The village follows when
its walker becomes one (#290).

**The record** (RON, D-035's format; `forge_mission::schema`):
- **An id** namespaced as content ids are (`lab:fetch`), and the schema's version.
- **Objectives:** an id and the text the player reads.
- **Places:** boxes and balls in the world, metres.
- **Items:** where each lies and how near the player must come to pick it up. An item is absent
  until a step spawns it.
- **One tree of steps:**
  - `Wait(trigger)`;
  - `Sequence`, `All` (Verse's `sync`), `Race` (the first done wins, the others are cancelled),
    `Branch` (in the background until the step holding it ends);
  - `Objective(id, step)`: shown while its step runs, completed when it is done;
  - `Spawn(item)`, `Succeed`, `Fail`.
- **Triggers:** `Entered(place)`, `PickedUp(item)`, `After(seconds)`, `Event(name)` (what the
  game sends: a kill or a door opened, until those systems exist), `AllOf` and `AnyOf`. Each
  part latches once it fires during its wait, so `AllOf` means each happened, not all at once.
- **Checked when read:** the version, unique ids, every id named defined, an objective named by
  one step only, finite numbers and positive sizes. Unknown fields are refused.

**The evaluator** (`forge_mission::Runtime`), at the fixed tick with the player's position:
- the steps compiled into a flat tree in their written order;
- each tick: the events the game sent are dated, the items in reach are picked up, then the tree
  is updated from its root, a step's children in their order;
- a step that ends at once lets the next begin in the same tick;
- out come the tick's changes (objectives started, completed or cancelled; items spawned or
  picked; won or lost) and where the waits under way point (`targets`), for an interface and a
  walker.

It is integer and `f64` comparisons in a fixed order, with no clock and no hashing by address, so
the same ticks and positions give the same state to the bit (D-016).

**Two saves:**
- **A snapshot,** exact: every step's phase, its start and a sequence's place, the latches, the
  items, the objectives, the events' ticks. It is for rollback, replays and digests. It loads only
  into the record it was taken from (the record's hash leads it).
- **A stage,** by id: the objectives completed, the items picked up, the outcome. It is for save
  games and hot reload, and loads into a changed record. The objectives done are skipped, the
  items stay picked, and the step in progress starts over, its timers too.

**In the lab:**
- **The mission joins the world's state:** saved, restored and digested with the rest.
- **Its items are drawn** as the lab's barrels, parked under the floor until spawned and once
  picked up.
- **The title** shows the objectives under way, or the outcome. Each change is logged.
- **`--mission-autopilot`** walks the player to the first target. Its walks are commands, so they
  are recorded and replayed.
- **The file is looked at every 30 frames** and read again when it changes, the mission keeping
  its stage. A record that does not read is reported and left aside. No watcher crate was added.

**Proved** (`demos/physics-lab/data/missions/fetch.ron`: climb the stairs, find the barrel that
appears, back at the start within 30 s, or fail):
- **The mission end to end:** climbed at tick 261, the barrel picked up at 627, won at 751.
- **Replay:** a recording of that run replays with its 40 digests matched and the same final
  digest.
- **Captures:** the barrel spawned (`lab-mission550`) and picked up (`lab-mission700`) are 0 px
  run to run and between the mesh path and the fallback.
- **Hot reload:** the file edited after the pickup (the return's text and its time): the mission
  reloaded with its stage (climb and find done, the barrel kept), the return started again with
  its new text, and the mission was won.
- **Cost:** 1.2–1.6 µs a tick on average, 6 µs at most, on the frame thread.
- **Unit tests:** 16, among them the race lost to its timer, a branch cancelled with its scope,
  `AllOf` remembering each part, an event counting only after its wait began, a snapshot
  restoring to the bit, a snapshot of another record refused, and a stage loading into an edited
  record.

**Not yet:**
- **Content packages:** D-035's `forge-data` is not built, so a mission is read from its file.
  The record is already RON with a namespaced id.
- **Kills and interactions:** no trigger of their own until those systems exist; they arrive as
  `Event`s.
- **Missions over `--net`:** refused for now.
- **An F1 zone:** the app's CPU zones are its own, so the evaluator's time is logged at exit
  instead.
- **Aliases** binding a record's places and items to what a generator placed. The research
  (`docs/research/missions.md`) weighs them.

**The research** (`docs/research/missions.md`, a cloud agent's note read after the build):
- **What it confirms:** the tree with Verse's operators (CD Projekt RED's *And* is `sync`, and
  its *Cut Control* cancels), objectives tied to steps rather than set by scripts, a definition
  kept apart from its state, and one total order inside a tick.
- **What it proposes next:**
  - **roles bound once when a mission starts,** as Bethesda's aliases and Hitman's ids. The
    generator names what it placed, and the record refers to those names;
  - **a stable id on every step,** with renames kept as tombstones, so that one *reconcile* rule
    serves hot reload and old saves at the step level. Today's stage works at the objectives'
    level;
  - **optional steps and a `fail_if` per wait,** with *cancelled* shown as *missed*, not failed;
  - **integer counters and the conditions `Near`, `Holds`, `Dead`, `CountOf`,** level-triggered,
    as the engine gets inventories and deaths;
  - **one-off interface cues keyed by mission, step and tick,** confirmed under prediction as
    Photon Quantum does.
- **Its patents:** nothing found reads on a record and its evaluator.
  - **Zynga's US 9,463,386 B1 and US 9,821,230 B2** (a state machine definition received from a
    server, setting graphical objects' states): read their claims before a server ever sends
    mission records to clients at run time.
  - **Kabam's US 9,463,376 B1** (objectives generated at random by task type, amount or time):
    read its claims before the generator picks those at random.
**The owner's answers to its nine questions** (§7.12, 2026-10-18): the proposals, the eighth
widened by the owner.
1. **Conditions on the world's state are the rule.** `Event` stays for what has no state yet
   (interactions, and kills until deaths are recorded), counted from its wait's start and kept in
   the snapshot. Each moves to a state condition when its system exists (`Dead`, `Holds`).
2. **A tie goes to failure:** when a wait's `fail_if` and its condition hold in the same tick, it
   fails.
3. **The first arm of a race to end decides it,** failed or not (Verse's rule). Arms ending in the
   same tick go by their written order, as now.
4. **The optional children of an `All`** are cancelled, shown as *missed*, when its required ones
   are done.
5. **Co-op:** a role standing for the players is inside when any player is. A per-player form
   waits for a game that needs it.
6. **Every tick,** measured (1.2–1.6 µs now).
7. **A required role that cannot bind** when an old save loads in a release: the steps that use it
   are cancelled (*missed*) and logged, and the mission goes on. An update never leaves a mission
   stuck, nor fails it. (The note proposed nothing; this was Claude's recommendation.)
8. **Two origins, one evaluator.**
   - **Generated missions** are made again on every machine from the map's seed. Only the
     generator and the seed travel.
   - **Authored missions** ship as records and Luau scripts in packages (D-035, D-072). This
     covers hand-made maps and games without a generator. The owner: some scripts make no sense
     with seeds, and a hand-made project's scripts are what it needs.
9. **Patents:**
   - read the Zynga claims before a server sends records to clients at run time;
   - read Kabam's before the generator picks objective types, counts or time limits at random.

**Version 2 of the record (2026-10-18, #287): the research's proposal with the owner's answers.**
- **A step is one record with named fields**, exactly one of these kinds:
  - a wait: `until`, and `fail_if`, tested first (a tie goes to failure); `fail_if` alone waits
    on `Never`;
  - `seq`, `all`, `race`, `branch`;
  - `do`: actions.
- **Any step may also have:**
  - `id` (else its parent's id and its place, `main/2`, the root's `root`);
  - `objective`, `optional`;
  - `then`, `on_fail` and `on_cancel` actions.
- **Optional fields are written without `Some`** (RON's implicit-some extension):
  `(id: "back", objective: "return", until: Inside("start"), fail_if: After(30.0))`.
- **Outcomes:** a step succeeds, fails or is cancelled.
  - a `seq` fails with a required step;
  - an `all` succeeds when its required steps have, and cancels its optional ones (*missed*);
  - a `race` is decided by the first step to end, failed or not;
  - a `branch` succeeds at once, its step running on until the step holding it ends;
  - the mission ends with its root step or an `End` action.
- **Conditions:** `Inside(place)`, `Near(place or item, metres)`, `Holds(item)`,
  `After(seconds)`, `Var(name, AtLeast | AtMost | Equals)`, `Event(name)`, `AllOf`, `AnyOf`,
  `Not` (tested as it is now, not latched) and `Never`.
- **Actions,** applied in order and seen by later steps in the same tick: `Spawn(item)`,
  `Set(var, value)`, `Add(var, delta)`, `Emit(tag)`, `End(Succeeded | Failed)`. The counters are
  integers declared with their starting values (`vars`).
- **Objectives are a view of their steps:** pending, active, completed, failed or missed. They are
  never stored.
- **The stage** keeps which steps ended and how, by id (followed through `renamed`, the record's
  tombstones), with the counters by name and the items. Loaded into a changed record, the ended
  steps stay ended without running their actions again, and the steps that were running start
  again. A branch whose step was running starts it again. This is the research's reconcile rule
  (`docs/research/missions.md` §7.8).
- **Version 1's records are refused** with a message: the one record there was (`fetch.ron`)
  was written again. Its race against a timer is now a wait with `fail_if: After(30.0)`.
- **Checked:**
  - 27 unit tests: v1's ported, plus the tie, the race decided by a failure, optional steps
    missed and failing, counters seen in the same tick, `End` cancelling the rest with its
    `on_cancel`, `Not`, `Near` an item only once it lies there, derived ids, renames in a stage,
    and a reload that runs no ended step again;
  - the lab's mission plays out as before: climbed at tick 261, the barrel picked at 627, won at
    751. `lab-mission550` and `lab-mission700` are 0 px against the accepted set on both paths.
    A recording replays with all its 15 digests matched;
  - 1.0 µs a tick on average, 6.9 µs at most.
- **Not yet:** roles bound to what a map or a generator placed (`Placed`, `Tagged`), `CountOf`
  over a role's members, and `Dead` or `Killed`. They wait for placements and deaths.

## D-074 — Decals in the resolve, from a world grid built once ✅ (proposed 2026-10-19, #304; the owner's go the same day: "follow your recommendations")

**The need** (#304): decals on static surfaces that blend with soft alpha, are lit as the surface
under them, share a few textures, cost little over a town of tens of thousands, and stay steady
under DLAA; at lower priority, a blend mask between two ground layers with edges per pair.

**Proposed** (`research/decals.md` §10):
- **Where:** projected box decals applied in the resolve's standard and layered classes, after the
  row's surface and before `sun_light`, so the sun, the probes, GTAO and the reflections treat them
  as part of the surface. No geometry: no z-fighting, soft alpha for free. The UV gradients are the
  box's axes dotted with the resolve's analytic `dpdx` and `dpdy`: anisotropic `SampleGrad`, no
  halos at depth edges (screen-space decals' flaw).
- **Which decals a pixel walks:** a world grid of 4 m cells built once on the CPU, each cell's list
  sorted by (priority, id), the non-empty cells in an open-addressed hash. No per-frame binning.
- **Two kinds:** row decals, a mask over a material row (D-007: a grass edge on a road shaded by the
  grass's own row, which physics and audio also read), in the layered ground first; and colour
  decals (albedo and alpha, an optional normal and roughness) everywhere. Normals blend as surface
  gradients (D-013's rule).
- **Textures:** 2-D array images, not an atlas (an anisotropic footprint reads past an atlas cell
  at grazing angles); masks R8, colour RGBA8 premultiplied in its mips; generated by Forge's own
  code first.
- **Then:** the ground's base/overlay/distance map per texel with a width and a height contrast per
  pair (item 4), decals at mirror rays' hits, texture bombing for grime.
- **Not now:** a DBuffer pre-pass (the fallback if the class passes' registers spill), mesh
  decals, decals baked into a virtual texture, decals on movers.

**Patents** (`research/decals.md` §9): one level of binning, built once, stays clear of
Activision's US 12,064,688 B2 (hierarchical decal binning, to 2042); masks or analytic stripes
rather than a volume rasterised per marking stay clear of Cesium's US 10,592,242 B2 (vector data as
screen-space decals, to 2039). AMD's US 10,803,655 B2 (per-tile light lists, to 2036) does not touch
the decals; D-065 already keeps it to read before Olsson's clusters for the street lamps.

**Decided** (the owner, 2026-10-19: "follow your recommendations for D-074 and D-075"), the eight
questions of `research/decals.md` §11 answered as recommended:
1. **Where:** in the resolve, from a world grid built once. A DBuffer pre-pass only if the class
   passes' registers spill; no virtual texture now.
2. **Kinds:** row decals (a mask over a material row) and colour decals, both in the first version;
   row decals on the layered ground first.
3. **Storage:** 2-D array images, R8 masks and RGBA8 colour premultiplied in its mips; block
   compression later, with D-047's.
4. **Textures:** generated by Forge's own code first, as the tileables and trims are; CC0 sets only
   on the owner's go per download.
5. **Patents:** as read: one level of binning built once (clear of Activision's US 12,064,688 B2),
   masks or analytic stripes rather than a volume rasterised per marking (clear of Cesium's US
   10,592,242 B2). The street lamps keep no per-tile screen light lists (AMD's US 10,803,655 B2):
   their rooms' and region's lists, or ReSTIR (D-065).
6. **The ground's blend map** (base, overlay, distance per texel): after the decals.
7. **The crossings** (#256): moved to decals if the A/B shows less shimmer than the ground pieces.
8. **Scope:** static surfaces only for now: nothing on movers, no bullet holes.

**Built, colour decals first** (2026-10-19, `forge_render::decals`): `MeshletSceneBuilder::
set_decals` takes a `DecalSet` (kinds and boxes); the standard, UV-mapped and layered classes
call `apply_decals` between the row's surface and its lighting. The grid's columns are 4 m (2 m
and 8 m both cost more in the village), at most 16 decals a column (32 since 2026-11-05), the
non-empty ones in an
open-addressed hash. A kind is one premultiplied RGBA texture for now, a separate image rather
than a layer of an array (the arrays come with the row decals' masks); a strip repeats along `u`
from a phase of its own per decal, so neighbouring walls do not repeat it. The city plan's kit
walls (street and court, where the ground storey meets open air, broken at doors, shopfronts and
passages) carry a strip of `textures::decal_grime` from 0.1 m under the ground to 1 m up;
`--no-decals` is the A/B. Cost in the village at 1600 × 900, three runs each way:
`shading/standard` 0.804 → 0.830 ms (the house view) and 0.890 → 0.925 ms (the junction from
12 m up).

**Stains and cracks on the paving** (2026-10-19): two stamp kinds, `textures::decal_stain` (a
ragged damp patch, its rim darker) and `textures::decal_cracks`, scattered over the city plan's
paved walking ground within the detail (the blocks' paved pieces and the streets' walking
ground), one stain to 40 m² and one crack to 150 m², each at a turn and a size of its own and
wholly on the paving (its middle and its corners: none spills onto a lawn); they mark the faces
that look up only. The village holds 1 210 decals with the grime (1 594 since its garden walls carry
the wall grime too). `shading/standard` without
the decals (`--no-decals`) and with them: 0.816 → 0.863–0.869 ms on the house front, 0.905 →
0.968–0.971 ms over the junction (two runs each way). In motion they add no shimmer: the
garden lane dollied at 1 m/s under DLAA, each frame's change against an `--ssaa` run's, differs
on 6.47 % of the pixels with them and 6.45 % without (20 pairs, the lane's foliage the rest).
**The stains reworked** (the owner, the same day: "what are the round shadow spots on the
ground?"): round, dark and even, the stains read as shadows. Now a stain is a faint, warm smudge
of dirt broken into irregular patches and streaks along its length (`decal_stain`: alpha at most
0.25, twice as long as wide, 1.4–3.2 m by 0.7–1.6 m), one to 90 m² of paving instead of 40. The
garden lane's two spots are gone; the paving only varies a little. Capture:
`reports/2026-10-19/village-paving-stains.png` (the old stains above, the new below).

**Row decals** (2026-10-19): a kind with a `row` (`DecalKind::row`) takes only its texture's alpha,
a mask over that material row: where it masks, the surface takes the row's colour and normal
(`standard_surface` of the row), as a layered ground's layer; the layered class alone draws them
for now (`apply_decals(…, rows)`), the other classes skip them. A strip's texture now starts at its
decal's own `Decal::phase`, so a run of strips along a curve goes on from one to the next (the
grime's phases are the same as before: the village's twelve views 0 px from the accepted set, and
`shading/standard` the same within noise). **The first use tried, and left out:** the island's
rivers' wet margins, a ragged strip of the lake bed's mud along each edge of each river (60 281
strips, `shading/layered` +0.04 ms at the mouth, +0.17 ms up the steep valley). One row does not
suit every bank: a dark mud line ran along the sand at the mouth and over the bars where two
rivers meet, and flat mud covered the steep stream's gravel. A river's edge wants wetness on
whatever its bank is (as the shore's swash), not another row; row decals wait for a feature drawn
in one material (a track, a worn path, a patch of another layer).

**Decals from a production's textures** (2026-11-04, #304): the private district's 10,914 decals
within 256 m (393 materials: dirt, dust, gravel, broken glass, road markings, posters) come as
block-compressed textures, which cannot be premultiplied in their mips, and many show one cell of
an atlas. A kind now says how its texture reads (`DecalAlpha`: premultiplied as Forge's own,
straight, its colour and alpha apart and multiplied as it is read, or its red channel a mask over
the kind's tint) and which part of it it shows (`DecalKind::uv_rect`, the gradients scaled with
it). The village's thirteen views are 0 px from the same build without it. Their materials' other
inputs (a gradient map's recolour, normal, roughness and parallax maps) wait for the colour decals
to be drawn there and compared with the game's pictures.

**The colour decals' normal maps** (2026-11-05, #304: "an optional normal", decided above, built
now):
- **The fields:** `DecalKind::normal` and `normal_strength`. The map is read as the kind's
  texture is (its atlas cell, a strip's repeats, the same gradients). Its red and green are the
  slopes along the box's `x` (the texture's `u`) and up the image (towards −`v`), as Forge reads a
  mesh's maps.
- **The bend:** both axes are laid in the surface's plane. The map's slopes, each over its z (so
  a texel tilts as the map says), times the decal's coverage (its alpha times its strength), add to
  a bend. A decal painted over another covers its relief as it covers its colour. Each class bends
  its final normal by it (`decal_bent`), after its own normal map, so the dents and cracks a decal
  carries catch the light whatever the surface's own map. The map is linear; its coverage is the
  kind's texture's (no decal of relief alone yet).
- **Without a normal map nothing changes, to the bit:** the bend stays zero and the normal is not
  renormalised. The village's thirteen views are 0 px from HEAD's build.
- **The private district:** 132 of its 329 decal kinds carry a normal map (40 of their 47 maps are
  in the archives). Its broken concrete and rubble now show relief under a low sun (0.02–0.08 % of
  a street view's pixels).
- **Still waiting:** the decals' parallax maps (the roughness maps: below).

**32 decals a column** (2026-11-05, #393): the private district's dense walls and pavements had
more than 16 decals over some 4 m columns. A decal is listed in every column its box spans, and
749 of those places were left out (a decal left out of one column and kept in the next is cut at
the 4 m edge). At 32, 77 are.
- **Cost:** on its benchmark path, `shading/standard` 2.225 → 2.234 ms, within run-to-run noise
  (the same path read 2.207 ms an hour before).
- **Elsewhere:** the village's thirteen map-check views log no column over 16, so its tables are
  the same.

**The colour decals' roughness maps** (2026-11-05, #393: "an optional normal and roughness",
decided above):
- **The fields:** `DecalKind::roughness` and `roughness_bias`. The map is read as the kind's
  texture is. Its red plus the bias, clamped to 0 to 1, is the roughness the decal lays where it
  covers (its alpha times its strength): a stain of wet blood glossy on dry asphalt.
- **Painter's order:** each decal's roughness covers what lies under it, as its colour does. The
  walk keeps the roughness laid and the share of the surface's own left, so the surface's
  roughness r becomes `r × kept + laid`. A decal without a map lays the surface's own back where
  it covers (a poster over a wet stain), as it covers the relief under it.
- **Where:** a UV-mapped row takes it before its roughness becomes a specular power, and so does
  each layer of a layered material (each walks the decals, #393's point 3). A row without UVs has
  only a power: `decal_power` turns it back into a roughness, lays the decals' over it and makes
  the power again, as the layered ground does over its blended power (added the same morning; the
  village's views unchanged).
- **Without a roughness map nothing changes:** `r × 1 + 0` in the UV path, and the power left as it
  is in the other.
- **The private district:** 114 of its 329 decal kinds carry one (24 of their 30 maps are in the
  archives), most with a bias (blood −0.7, asphalt +0.25, plaster +0.2). Its wet patches and stains
  catch the sky: 3.3 % of an alley view's pixels change, 1.5 % of a street's.

**Puddles mirror** (2026-11-05, #395): a puddle decal on rough ground showed no reflection.
- **Why:** a pixel traced a mirror ray (#52, #306) only when its material row was smooth
  (`wants_mirror_ray` reads the row's power). A decal's roughness lowers the pixel's inside the
  shading, after that choice, so a puddle took the probes' dimmed sky alone. The private district's
  puddles (their source's `decal_puddle.mt`) also laid their roughness at their colour's strength,
  0.3 to 0.9, their source's `DiffuseAlpha`.
- **The change:**
  - `DecalKind::roughness_strength`: the share of its coverage its roughness takes. None keeps its
    strength, as before.
  - A standard pixel whose colour decals made it smoother (`LightInputs::laid`, their share
    where they lowered its roughness) and whose final power reaches the mirror rays'
    (`RAY_REFLECTION_POWER`) traces one. Its traced share is scaled by that share, so a puddle's
    edge fades from the probes' sky to its ray's, and a decal that roughens a glossy row leaves
    no ring (the review). It traces only where its ray carries half its reflection or more,
    fading in up to 0.8 (`DECAL_MIRROR_LEAST`, `_WHOLE`): the district's glossy paint decals
    (logos, graffiti, roughness 0.3 to 0.4) otherwise took rays for a few percent of theirs, and
    cost 0.5 to 0.7 ms. Its roughness rides in the request image's spare channel, so
    `reflections_main` traces at the puddle's roughness, not the row's.
  - Where the reflections pass will visit a tile, each standard pixel that asked for no ray gets
    a request of none, so the pass never reads an earlier frame's.
  - The standard class alone: the layered ground and ice keep their rows' reflections.
  - `MeshletRenderer::decal_mirrors`, `r.reflections.decals` (on): the A/B. Only a scene with a
    decal kind that lays a roughness map turns the frame's option on; elsewhere the shaders take
    their old paths, to the bit.
- **The private district** (its puddles' roughness at their full mask):
  - **Frame 30:** its puddle mirrors the gap of sky between the buildings, as the source's does.
    The puddle's mean luminance goes from 3.26 EV under the source's to 0.15 under. Without the
    rays it read as grey ground.
  - **The 14 frames** with the rays against without, by geometric mean, median and mean:

    | rays | geo mean, rms | median mean, rms | mean mean, rms |
    |---|---|---|---|
    | off | +0.02, 0.51 | +0.02, 0.61 | −0.05, 0.48 |
    | on | +0.01, 0.54 | −0.07, 0.70 | +0.00, 0.44 |

    Frame 30 moves alone: −0.81 → −1.02 (geo), −0.42 → −1.41 (median), −0.88 → −0.33 (mean); with
    the traced share scaled at the edges, −0.93, −1.07 and −0.45 (and the long tubes as flat
    lamps, the viewer's since: the 14 frames +0.00, 0.54; −0.03, 0.68; −0.03, 0.45).
    Its puddle's dark reflections (the alley's walls) are darker than the source's dark puddle.
    The whole frame sits 0.8 EV under the source's, and our camera there looks less steeply down
    than the source's.
  - **Motion:** the share of the alley clip's ground pixels that flash or dip for one frame stays
    2.3 %.
- **Cost** (the district's benchmark path, 2560 × 1440, DLSS Quality, interleaved after a warm-up):
  - **Every decal-smoothed pixel traced:** the frame 15.63–15.74 → 16.28–16.49 ms (the rays
    contend with the probes' rays on the async queue: `gi/probe rays` +0.28, `shading/standard`
    +0.16, `shading/reflections` +0.14).
  - **From half their reflection:** 15.67 and 15.81 ms without, 15.71 and 15.86 with;
    `shading/reflections` 0.22–0.24 → 0.26, `shading/standard` +0.03.
  - Only a scene with a decal kind that lays a roughness map pays either.
- **Elsewhere:** the village's decals carry no roughness map, so its option is off: its thirteen
  map-check views are 0 px from the build before.
- **What smooths a surface fills its relief** (2026-11-06, `decal_fill`): the mirror rays over a
  bumpy ground's normal map each went their own way, so a puddle read as a field of sparkles.
  - **Now:** a decal's roughness flattens the shading normal towards the surface's own, by how far
    it took the roughness down, full from a drop of 0.5 (`DECAL_FILL_DROP`). The normal map's
    variance, the decals' slopes under it and a layer's microblend bumps go with it, on every class
    that takes the decals' roughness.
  - **First built by coverage alone, and wrong (the review):** any drop counted in full, so a logo a
    little smoother than its wall lost the wall's relief, and a ground's map to either side of a
    puddle's value flipped from texel to texel.
  - **Checks:** the gate; the village's thirteen views score what they scored before against the
    accepted set. In the private district frame 30's puddle is a flat mirror, and the 14 matched
    frames don't move (geometric rms 0.44).

*(research: decals.md; D-007, D-013, D-021, D-026, D-028, D-047, D-065; issue #304)*

## D-075 — Forests as a ladder per tree, grass as a GPU pass near the camera ✅ (proposed 2026-10-19, #302, #300; the owner's go the same day: "follow your recommendations")

**The need** (#302, #300; the owner, 2026-10-18): forests whole and seamless as the camera moves,
no visible pops; grass like modern games' carpets near the camera, placed from the map's data.
D-013 chose the ladder (cards, then impostors far) without building its far end.

**Proposed** (`research/forests-and-grass.md` §8):
- **First, with no decision needed:** each BLAS split into an opaque geometry and a masked one.
  Today one cut-out row forces the whole instance non-opaque, so a ray through a tree tests its
  wood and cores in the candidate loop as well as its cards. Core Vulkan, every vendor.
- **The ladder per tree,** cooked from `forge_flora`'s graph: cards near (today's `grown_mesh`),
  closed crown shells mid (they simplify in the cluster DAG, as #297's proxy shells do), a
  hemi-octahedral impostor far (base colour, alpha, normal, depth; its own shading class). Chosen
  per instance in the cull by projected size, with a per-instance offset and hysteresis.
- **The crossing:** both levels drawn over a band, each pixel keeping one by the complementary
  dither of `spatio_temporal_noise`, which repeats with DLAA's 8 phases (a still view settles in 8
  frames). Pixar's stochastic-LOD patent (US 6,300,956 B1) expired in 2018.
- **What the rays see:** a fixed representation per tree, so the static TLAS never changes with
  the camera. Probes and mirrors meet #297's proxy. Shadow rays meet the cards near the camera and
  the proxy beyond, chosen by the ray's origin through two TLAS instances and two mask bits (#301).
- **Grass:** a GPU pass of opaque blades from a cached density map per region, within about 35 m,
  drawn into the visibility buffer by mesh shaders, rebuilt bit-exactly in the resolve
  (`PRECISE_ENTRIES`, `lerp_exact`), at least a pixel wide with the missing coverage dithered, met
  by no ray; the ground's grass layer beyond.
- **Optional:** `VK_KHR_opacity_micromap` for the cards where the device exposes it, the any-hit
  loop as the fallback.

**Patents** (`research/forests-and-grass.md` §7): Pixar's US 7,973,789 B2 (to about 2028-11-10)
claims representing a model by a modified subset of its elements (fewer, larger leaves or blades:
Nanite's Preserve Area, AMD's widened grass); the proposal keeps counts fixed until the owner
decides. Sony Online's US 8,115,765 B2 (to about 2028-01-07) claims flora generated by layered
rules around the avatar; the grass pass reads a cached map and evaluates no rules around the
player. NVIDIA's micromap traversal (US 12,475,631 B2) is left to the driver; Intel's
comparison-mask LOD in rays (US 11,989,817 B2) is not used (plain AND masks per ray).

**Decided** (the owner, 2026-10-19: "follow your recommendations for D-074 and D-075"), the eight
questions of `research/forests-and-grass.md` §9 answered as recommended:
1. **The crossing:** a complementary dither DLAA resolves, both levels drawn over a band; the
   per-lump crossing inside the cluster DAG later, once #303's authored parents exist.
2. **Pixar's US 7,973,789 B2:** counts stay fixed until it expires (about 2028-11-10): no thinning
   of leaves or blades with the survivors enlarged. The ladder hands over to other
   representations (closed shells, impostors) instead, and #303's free borders wait.
3. **The grass pass:** inside D-063's rule. The blades are a cached function of each region's map,
   drawn within a distance of the camera; no rule is evaluated around the player (Sony Online's US
   8,115,765 B2 claims that).
4. **Opacity micromaps:** allowed later as an optional speed-up where the device exposes
   `VK_KHR_opacity_micromap`, Forge's own baker, the any-hit loop the fallback; traversal left to
   the driver (NVIDIA's US 12,475,631 B2).
5. **Grass met by no ray:** yes. It casts no ray-traced shadow, and the probes see the ground's
   grass row.
6. **Grass bent by walkers:** after the carpet looks right.
7. **Impostor memory:** a quarter of Fortnite's 6.8 MB a tree: 1024² base colour and alpha (8 × 8
   frames of 128 px) and 512² normal and depth per species and age class, block-compressed when
   D-047's compression lands; raised if the impostors read blurry at their crossing.
8. **The far canopy** (D-013's band beyond 600 m): for the island and the planet later, not the
   village maps.

**The order of the work** (§8.5): the BLAS split (done, #307); the ladder record and the
per-instance crossing; two TLAS instances per tree with the shadow-ray bit; the impostor baker
and its shading class; the grass pass; micromaps last.

**Step 2 built** (#302, 2026-10-19), behind city-plan's `--tree-ladder [PX]` until the owner had
looked:
- **The record:** `MeshletSceneBuilder::set_ladder(mesh, next, size_px, band)` names a mesh's next
  step, the projected size of an instance's sphere (output pixels across) at which it is half
  crossed, and the band (a share of that size either side). The instance's sphere grows to hold
  the next mesh's, about the same centre (the culls' LOD tables are measured from it). Since
  2026-10-22 (#332, a game's full tree → stand-in → far step) the next mesh's own ladder is
  followed, up to three crossings: past a crossing an instance measures itself against the next
  step's size and band, and the culls write `2 k + t` (the pair `k` steps down, its crossing
  `t`), which the cluster cull, the cut-outs' raster and the resolve read. The far end is given
  first, so each sphere holds the ladder below it; one-step ladders draw as before (0 pixels),
  and a three-step chain draws the same on both paths with clean validation.
- **The choice,** in both instance culls (`ladder_crossing`): the sphere's size, taken up to 15 %
  off by the instance's id, against the band. The crossing `t` (0 the mesh, 1 the next) is
  written per instance (`Frame::ladder`, a transient); the instance appends its mesh's work, the
  next mesh's (tagged `INSTANCE_LADDER_BIT`, which the cluster cull reads to pick the mesh), or
  both. Every list keeps its cull order, so the ids, and the images, stay the same from run to run.
- **The crossing:** a crossing instance's clusters of both meshes take the cut-outs' raster
  (`CUTOUT_LADDER`), where each pixel keeps its mesh's where `spatio_temporal_noise(..).x ≥ t`
  and the next mesh's below: no hole and no double, averaged by DLAA's history. The resolve tells
  the two apart by the cluster's index (outside its instance's mesh's run: the next mesh), so the
  two meshes must agree on their texture coordinates and neither be skinned.
- **The rays** keep meeting the mesh itself (the cards), as decided.
- **The village's plants:** each grown plant cooks its crown's shells (`grown_shells`, a row of
  their own at 0.55 of the leaves' colour: at the leaves' own a far wood stood 1.75 times as
  bright as its cards). The crossing is set where the shells' lumps stand `PX` pixels across (6 by
  default, `GrownPlant::lump`): measured on the sphere alone, the shrubs crossed at 7 m.
- **Measured** (the village from the air, 1600 × 900, DLAA): the frame 4.69 → 4.34 ms (meshlet
  pass 1 1.30 → 1.10, cluster cull 1 0.51 → 0.34); on the ground, where more trees cross than have
  crossed, 4.81 → 4.72 and 4.43 → 4.46 (pass 1 +0.08 ms: both levels and the cut-outs' raster). A
  still camera's frames 10 apart differ on 0.06–0.07 % of the pixels with the ladder, 0.08 %
  without. The A/B harness with the ladder on (`--no-occlusion`, the fallback, `--show-culled`)
  differs in 0 px; without it every image is the accepted set's. The far woods, whose cards the
  cluster LOD had thinned to bare twigs, stand whole (`reports/2026-10-19/village-tree-ladder.png`).
- **Left:** the look of the shells for the owner (smooth lumps, one colour per tree), timings on
  the serial frame.

**Step 3 built** (#302, 2026-10-19), with the ladder (`--tree-ladder`; `--far-shadows M`, 0 off):
- **A world of rays of its own:** `RAY_SHADOW_FAR`. An instance's `Visibility::far_shadows` says
  whether it is in it. A plant's cards are not; its shadow proxy (`grown_shadow_proxy`: its wood,
  each lump a closed shell at 0.8 of its radius, opaque) is in it alone (`Visibility::FAR_SHADOWS`).
  Every other instance is in both shadow worlds.
- **The switch:** a shadow ray (the sun's, in its trace and the resolve, and the lamps') from a
  point `MeshletSceneBuilder::set_far_shadows` metres from the camera and beyond (60 in the
  village) meets the far world. It is dithered by the pixel over the next quarter of the distance,
  which SIGMA averages.
- **Measured** (the village with the ladder, 1600 × 900, DLAA):
  - **From the air:** `shadow/trace` 0.22 → 0.11 ms, the frame 4.15–4.24 → 4.04–4.09 ms.
  - **On the main street,** where the shadows fall within 60 m: unchanged (0.35 ms).
  - **The images:** the far trees' shadows keep their shapes, their outlines a pixel off (air view:
    ꟻLIP mean 0.021; the main street 0.5 % of pixels, ꟻLIP 0.008).
- **The proxies are placed with the ladder whatever the distance:** each tree's two tones follow
  its instance's id, and placing them only with far shadows reshuffled every later tree's tones in
  the A/B.

**The grass pass built** (#300, 2026-10-22), behind city-blocks' `--grass [M]` (50 m) until the
owner has looked (`forge_render::grass`, `shaders/meshlet.slang`'s "grass"):
- **The map:** square cells of 0.5 m (`GrassMap`), each with the ground's level and its blades'
  count (at most 128), height and row, made once from the region's data. The village paints its
  gardens' lawns (56 blades a cell, 12 cm) and its parks' grass (80, 22 cm; 40 and 56 before the
  owner asked for a little more density) from the ground tiles, cleared under the houses and
  along the garden walls. A blade is a function of its cell
  and its index there: placement, height, lean, facing and width from a hash, nothing evaluated
  round the viewer. Each cell also keeps the ninths of it no blade stands in, painted and
  cleared at a ninth's precision: the lawn stops within 8 cm of the paving (the owner saw blades
  on it, a border cell's spreading up to 25 cm past the lawn's edge).
- **Its ids:** the visibility id's top bit, the visible list capped at 2^24 slots for it:
  `cell << 10 | blade << 3 | triangle`. The resolve, the sun's shadow trace for SIGMA and the
  lamps' passes rebuild a blade's triangle from it alone, from the map uploaded once; the
  reflections and the movers' motion skip it.
- **The culls,** in a fixed order: each cell of a window round the camera counts its patches of
  16 blades within the reach and the frustum, one workgroup scans the counts, and the cells list
  their patches. The same ids, and images, from run to run.
- **The raster:** a mesh-shader workgroup a patch (7 vertices and 5 triangles a blade), or one
  non-indexed draw on the fallback path, both compiled precise. A blade under a pixel wide is
  drawn a pixel wide and its covered share kept by the frame's noise. Over the last fifth of the
  reach the blades sink into the ground's own grass.
- **Far** (the reach 50 m; the owner saw the curve where a 20 m reach ended): every blade of a
  cell stands out to the reach. A first try thinned them from 12 m with the rest widened
  (7148e1d0), which decision 2 below rules out while Pixar's US 7,973,789 B2 runs: taken out the
  same day. At 50 m the four passes take 0.40 ms (0.33 with the first counts, 0.13 at 20 m), the
  turn's flicker 3.5 %, the band from 17 m to the lawn's far end 0.1 %.
- **The look:** the blades take their cell's row (the lawn's, the park's) and one of its two
  colours a clump, darker towards the root; their normals rounded across them. With distance,
  their normals and contrast converge to the clump's mean, fully by 20 m or the reach if nearer
  (a reach's distance alone let a 50 m reach's blades shimmer twice as much), and at the reach
  their brightness to the ground's they sink into, which took the visible flicker of a turn under
  DLAA against a supersampled reference from 17 % to 6 % of the grass's pixels.
- **The wind** (`--grass-wind S`, 0.15 by default): a gust's wave rolls across the field along the
  wind's way, each blade a little out of step, with a flutter of its own; the tips lean `S` of
  their height at most. "grass/motion" gives the blades' pixels their motion (the blade rebuilt
  at the frame before's sway time, as `mover_motion_main` does a mover), so DLAA keeps them:
  against a supersampled frame the swaying grass is as close as the still (ꟻLIP 0.0465 and
  0.0474 on the grass). 0.015 ms.
- **Not yet:** the walkers' bending, a measured fit of the colours to the ground beneath.

**Step 4 built: the impostors** (#302, 2026-10-22, the plan in #302's comments), behind
city-plan's `--impostors [PX]` with `--tree-ladder` until the owner had looked (on by default since 2026-10-25, below); `--impostor-bake
DIR` writes the atlases as images (`forge_render::impostor`, `shaders/impostor.slang`,
`impostor_layout.slang`, `impostor_draw.slang`):
- **The layout:** the upper hemisphere unfolded into a square, a diamond turned to fill it (R.
  Brucks), 8 × 8 frames each baked from the direction at its middle (the lowest 7° above the
  horizon), a view blending the three frames around its point by barycentric weights; the frames'
  axes (`frame_axes`) never upside down.
- **The bake, before the run's first counted frame:** one kind for each species and age class
  (the age's last seed; the village's 27), one after another, a frame of a bake each app frame.
  Meanwhile the app counts no frame (`Context::preparing`: captures, frame limits and the run's
  clock wait, the screen black; since 2026-10-29 an interactive run's loading screen covers it,
  saying "Baking impostors n/N"), so a capture is the same whatever the bake took: 1.4 s for the
  27. Kept in D-053's derived cache (`impostor-blocks`, every level's blocks as drawn, 12 MB on
  disk for the 27), keyed by the cooked mesh, its rows, its textures' inputs and the code that
  bakes (`crates/forge-render/build.rs`: the bake and its encoder, the resolve, the materials,
  the textures): a start that finds a kind copies its blocks into its atlases in one frame, the
  27 in 0.33 s, the image the same to the pixel; the fresh bakes are stored on a thread of their
  own.
  One renderer for all, its resolve compiled from `meshlet_bake.slang` (`CullFlags::BAKE_ALBEDO`,
  `BAKE_NORMAL`: `sun_light` returns them, the leaves' light through adds nothing; in the frame's
  own entries a branch never taken showed #349's unshaded tiles). Each frame drawn at full detail
  from 1000 radii off with a field that fits its sphere (near enough to parallel), at 256², taken
  4 × 4 into the frame's 64² square: the base colour weighted by coverage and premultiplied by it
  (its alpha); the normals averaged and left that long, shorter where the leaves spread, so lit
  by them a texel gives its leaves' average light (made unit, a crown's spread leaves lit as one
  face, 3 % brighter); beside them the depth, the surface's distance from the middle towards the
  frame's camera in radii; and the leaves' Fresnel weight towards the frame's camera, Schlick's
  (1 - n·v)⁵ of each of the 16 render texels averaged (since 2026-10-25, below: it replaced the
  occlusion, each texel's share of the sky read from every frame as a shadow map). Then the mips.
- **Memory:** frames of 64 px, not question 7's 128 for the base colour. An impostor begins under
  50 px across (its fade's size and band), so a frame still gives each pixel more than a texel,
  and a kind for each age (its own proportions: the oldest stretched to a young plant's crown read
  squashed, the hedges' shrubs flat) costs less than one a species at 128. Baked in half floats
  (6.7 MB a kind with mips, 180 MB for the village as first built), then block-compressed on
  the GPU (D-047; `shaders/impostor_compress.slang`): the base colour in BC7 (its colour in
  sRGB), the normals in BC7, the Fresnel weight (first the occlusion) and the depth in BC5, a BC4
  block each. 1.0 MB a kind, 27 MB for the village; the half floats are one set shared by the
  bakes and freed after.
  - **The encoder:** BC7's mode 6 alone (one subset, 4-bit indices), its endpoints at the
    block's extremes along its principal axis, then refitted by least squares (0.1–0.3 dB more);
    BC4 from each block's least and greatest value. First the depth shared the normals' alpha
    in one BC7 block (ꟻLIP 0.009 in the air view); moved beside the occlusion, the normals fit
    their colour alone and the depth gains 14 dB.
  - **Against the half floats** (`imgdiff`): ꟻLIP 0.007 from the air (2 % of the pixels, the
    far crowns' dithered edges) and 0.002 in the street views; drawn ten times their size
    (`--tree-ladder 40 --impostors 400`) 0.006–0.007, the crowns' shading a little grainier.
    The atlases' PSNR over the 27: base colour 40 dB (the worst 36), depth 48 (42), occlusion
    36 (34); the normals about 30 by the review images, whose straight values carry the
    coverage's error at the crowns' edges. The draw got cheaper: 0.19 ms from the air (0.21),
    0.05 in the street (0.07).
- **The crossing:** the shells fade in turn (`set_fade`, the ladder's last step), and an
  instance's impostor draws where their noise leaves their pixels: the same crossing measured the
  same way (the instance's culling sphere and id, the shells' size and band, and not before the
  cards' crossing to the shells has ended), the complementary pixels. A quad facing the camera, six
  vertices an instance, every instance each frame (one not crossing moved off screen by its vertex
  shader).
- **The draw: a forward pass after the resolve,** before the sky's compose hazes it with the rest,
  not a shading class of the resolve (a change in the resolve's entries shows #349). Its depth is
  written where the bake saw the leaves, so a plant meets the ground and its neighbours where it
  stands; its motion with it when the motion vectors came before (with SIGMA), else that pass
  takes it from the depth. The quad stands at the front of the sphere and no depth written is
  nearer (`SV_DepthLessEqual`), so the depth test rejects hidden plants before their shading:
  from the air the frame is 0.11 ms shorter (4.03 → 3.92 ms, the pass 0.21), on the main street
  the same (the pass 0.06 ms; 0.14 with a centred quad).
- **Its light, as the resolve lights a rough row:** each plant its own tone (its id's mix of its
  leaves' two colours, `hash_color`), the sun through one shadow ray from the leaves' point into
  the far shadows' world (its own proxy and its neighbours', as the far cards' rays), the point
  moved along the view by the frame's noise so that DLAA averages it as SIGMA does the cards'; the
  sky's irradiance, and its reflection (the row's Fresnel and roughness) dimmed by the baked
  occlusion (since 2026-10-25 the probes' light and the leaves' own Fresnel weight, below); the
  highlight and the light through the leaves. Each term
  measured against the cards from the air (green, sRGB, by the square): lit without a shadow, 107
  against the cards' 89 (the shells' tone, 0.55 of the leaves', answers the same); without the
  reflection, too green (blue 41 against 54); with its own frames as its shadow map in place of
  its proxy, 98 on the young street trees. With all of it, 90.7 against 89.2, and the far woods
  67.3 and 76.4 against the shells' 66.8 and 80.7.
- **The checks:** the mesh path against the fallback, `--no-occlusion` and
  `--instance-occlusion on|off` 0 px with the impostors; validation (synchronisation included)
  clean on both paths; a still camera's frames 10 apart differ on 0.51 % of their pixels (the
  cards 0.53 %, the shells 0.37 %: the coverage's dither at the edges, as the cards' cut-outs).
- **The crowns' contrast** (the same day). From the air, tree by tree, the impostors spread wider
  than the shells and the far cards (the deviation of 4 × 4 blocks' green, the means equal: 13
  against 7–9). Against the cards at full detail (the green's view, every tree past the ladder an
  impostor) they spread as the cards do (17.8 and 17.5 against 20.1 and 16.2): the far cards are
  thinned by the cluster LOD, and the shells lit each lump as a sphere of its own. The owner's
  choice: the shells round, their normals bent out from the crown's middle as the cards' are
  (`grown_shells`), so the three steps light a crown alike; the shells' means moved under 2 %, the
  tone kept. Two more of the impostor's own: the bake at each row's mean tone (one instance's tone
  was baked for the species), each plant's tone on its leaves alone (the leaves' ratio turned the
  trunks pink); and the occlusion on the reflection alone, the sky's diffuse light undimmed as on
  the shells and the far cards (the crowns in shade read near black: the darkest tenth 60.5 →
  64.4, the shells' 69.6). Left: the brightest tenth by the square, the young street trees'
  sunlit crowns, 115 against the shells' 95 and the cards' 100, lime beside them. Not the cause,
  measured: the bake's colours (each species the same at every age), the shadow ray (without it
  12) and its noise, the leaves' depths in a texel, the normals' spread (they agree), the sky's
  direction.
- **Not yet:** the bright young crowns above, each age's other seeds, and the default (the
  owner's look first). Block compression is built (above, "Memory").
- **The owner's flight in** (2026-10-25, `--tree-ladder --impostors` flying forward: trees
  "changing colour very strongly", crowns "almost empty" then full, small trees "popping into
  existence"):
  - **The clouds' shadow on the impostors' sun** (`cloud_light`, as the resolve reads it): they
    stood in full sun under the clouds' shadow the cards and the shells were in. The square's
    trees at 0.25: G 91.6 → 81.5, the cards' 81.2; at the default hour 84.7, the cards' 84.7;
    noon unchanged. Their shadow ray also meets the movers' structure, as the resolve's do: a
    plane or a ship passing over shades the far woods as it shades the near trees (the owner's
    question).
  - **The crowns kept whole in the cluster LOD** (`CookOptions::keep_sections`,
    `flora::CROWN_SECTIONS`, #303's "keep the finest level"). The cards never simplify, but the
    cores did and shrank to nothing, so a far crown thinned to specks before its shells and
    impostor, made from its finest look, filled it out again; a young tree's faint cards then
    gave way to a full dark ball. Now the cards draw every core and card to the crossing: the
    meadow's young tree, its impostor G 78.5 against its cards' 77.8. The cost (`docs/PROFILE.md`):
    0.2–0.5 ms with the ladder, 0.9 ms from the air without it (the far woods' cores drawn, which
    also read darker, as they do near).
  - **The shells' tone per channel** (`CROWN_SHELL_TONE`, 0.55 → 0.80, 0.74, 0.95 of the
    leaves'). Against the cards kept whole, over the shells' own pixels (`--show-ladder` as the
    mask) in four views, the shells stood 8–11 % darker in luma and greener (red and blue 13–14 %
    low: the cards' gaps show the ground and the sky). Now within 1 % at the default hour and
    the morning's (the air view 60.0 against 59.7, 59.3 against 59.4), 9 % bright at noon, where
    the cards' crowns hold more inner shade than round lumps do. The shells don't shrink under
    the cluster LOD as the cores did (0.6–0.9 % of pixels move with it off).
  - **Left:** a young tree's impostor still 12 % brighter and greyer than its cards (R +24 %,
    B +28 %, the meadow's view), the "bright young crowns" above. Dimming the impostors' sky by
    their baked openness fixed the nearest crowns in shade but darkened the far woods, and made
    no difference at the real crossing (40 px), so it is left out.
- **On by default** (the owner, 2026-10-25: "yes make it the default"): city-plan draws the
  ladder at 9 px (6 at first: the owner, flying back, "the trees become almost transparent", the
  cards a pixel or two each, speckled dark over the water; at 9 the riverbank's row stands whole,
  the meadow view 4.88 → 4.11 ms; at 12 the meadow's trees dulled to shells from the air) and the
  impostors at 40 px without flags; `--tree-ladder 0` draws the cards all
  the way, `--impostors 0` keeps the shells (`Args::tree_ladder`, `Args::impostors`). The far
  shadows' world comes with the ladder (60 m). The frame from the air 6.25 → 4.78 ms against the
  cards kept whole (`docs/PROFILE.md`); the impostors' bake adds 1.4 s to a first start, 0.33 s
  from the derived cache.
- **The far woods' colour** (the owner, 2026-10-25: "a lot of color changes in the distant
  forest", "the impostor ... far often looks a bit darker"). Measured at a fixed exposure (the
  metered one moves with what the impostors draw), over the tiles the impostors cover, in the
  owner's three views and the air view: the impostors stood 1.5–6 levels below the shells they
  take over from in red and blue, and 0–3 in green, so greener and darker. Taking the leaves'
  reflection out of the cards and the impostors alike (a scratch resolve) closed the gap to half
  a level: the cards reflected 1.65–1.8 times the impostors' sky. Two causes, two changes:
  - **The leaves' own Fresnel weight, baked.** The cards' leaves keep their normals bent out from
    the crown's middle on both sides, so those at its edges and those seen through its gaps face
    away and reflect the most; the impostor took Schlick's term at the leaves' averaged normal,
    which faces the camera (the term is convex: the average of the leaves' is larger). The bake
    now averages each leaf's into the BC5 atlas the occlusion held, and the reflection is no
    longer dimmed by the occlusion (the far cards' is not: the screen's occlusion is weak that
    far). Alone it overshot by a fifth: the cards reflect the probes' light, darker in a wood.
  - **The probes' light, once a tree.** The vertex shader looks the probes up at the front of the
    tree's sphere for a normal up and one down, over the open sky's; the sky's light and its
    reflection are dimmed between the two by the normal. Per pixel the same lookups cost the pass
    0.17 ms more for no visible gain (a far crown is smaller than the probes' 16–32 m spacing);
    once a tree, 0.03–0.05 ms.
  - **After:** within about a level of the shells in every channel (red, green, blue: the
    flight-in view +1.0, +0.9, +1.2; the meadow +0.4, +0.1, +0.8; the riverbank +0.4, +0.5,
    +0.6; the air view +1.3, 0.0, +2.3), where they were −1.8, −1.5, −2.8 / −3.7, −3.1, −5.0 /
    +0.2, +0.7, −0.9 / −6.4, −5.8, −7.9. Tiles of 40 px off the shells by 1.5 levels or more at
    the default exposure: 41 → 26, 92 → 51, 76 → 30. The bake lost its occlusion pass.
    Tried and left out: the probes per pixel (above), the clamped cosine over the leaves' spread
    for the sun (no change: the baked normals agree within a texel), the sky-view table in the
    mirror direction (the leaves are rough enough that the resolve takes the irradiance alone).
  - **Left:** against the cards (not the shells) green stays 1.5–3 levels high on the darkest
    tiles, as the shells' own; the shells were fitted to the cards in luma.

**Micromaps built** (#333, 2026-10-23, question 4's answer), on by default in city-plan, where
the device has them (`MeshletSceneBuilder::set_opacity_micromaps`, city-blocks' `--omm`,
`FORGE_OMM=0` or `1` overriding; `forge_render::opacity`, `Device::build_opacity_micromap`):
- **The extension:** `VK_EXT_opacity_micromap`, the KHR one's EXT predecessor. ash 0.38 has only
  its names, and with the EXT enabled a ray query uses the maps without Slang declaring
  `OpacityMicromapIdKHR`. The RTX 5070 Ti exposes both; a device without either (the RDNA 2 iGPU)
  builds no maps, and the loop stays the fallback.
- **Forge's own baker**, on the CPU at the scene's build:
  - Each not-opaque triangle of a cut (#307's second geometry) is divided down to micro-triangles
    about 2 texels on a side, at most level 9, in 32 MB in all.
  - Each micro-triangle is transparent or opaque only where every alpha its bilinear taps can
    read at level 0 (the box of its texels and one past) lies on one side of the cutoff, by
    1/512. Elsewhere it is unknown, and the loop's test decides, so the picture is the same.
  - A triangle wholly in one state takes a special index. Equal maps are stored once.
  - A cut whose triangles all come out unknown gets no maps. Built with them anyway (the
    houses' glass), its structure moved up to 800 px of a window in the garden view.
  - The micro-triangles' order is the specification's reference code, ported (CREDITS.md).
  - No NVIDIA OMM SDK code is used. Traversal is the driver's, as question 4 asked (NVIDIA's US
    12,475,631 B2).
- **Which instances keep them** (`tlas_instances_main`):
  - One whose cut-out rows read as the rows the maps were baked from (`same_ray_alpha`,
    `opacity_rows_match`: the fields the loop's tests read) keeps them. Any other turns them off
    (`DISABLE_OPACITY_MICROMAPS`, the structure built to allow it).
  - The probes' rays keep a quarter of a cut-out's triangles by a hash (`probe_leaf_kept`),
    which an opaque micro-triangle would bypass. A mesh they see therefore gets maps with no
    opaque cut-out state (the clear parts still skipped), and an instance they see turns off
    maps that have one.
  - The loops that force non-opaque (the jelly's and the ice's crossings, the rays leaving a
    body) see an opaque micro-triangle as a candidate, by the specification's order: micromap,
    then instance flags, then ray flags. A transparent one is skipped by every ray, the forced
    opaque ones too (the dust's, the particles' and the splashes' sun rays now see through a
    card's clear texels).
- **Checks:**
  - The forest, the green, the garden and the house: 0 px at frame 240.
  - The map check's other views: within their run-to-run noise (20–81 px base against base;
    with the maps 7–96 px, the same scattered pixels).
  - Sponza with `FORGE_OMM=1` (its plants seen by the probes, so maps without opaque states):
    1 px by 3 levels.
  - Synchronization validation: silent.
- **Measured** (`docs/PROFILE.md`): the sun's rays (`shadow/trace`) 10–21 % cheaper, the frame
  0.05–0.13 ms faster in five views of the village.

**The cards' alpha test one mip finer (2026-10-29, #363).** The owner saw trees lose their
leaves at one distance on the way in, fine nearer and farther. The young holm oaks of the park,
200 m off from the air, kept 2–5 % of their pixels on their cards (their dark cores' shade
through them), against 61–73 % as their shells and impostor; supersampled 2 × 2 or drawn at
3200 × 1800, the same cards were full and bright. The cut-outs' alpha test (`cutout_kept`) now
samples one level finer than the colour (`CUTOUT_MIP_BIAS`): 55–63 % (mip 0 the same), so a
crown no longer thins as it hands over to its cards. On the walk in from the start at noon the
far woods' cards came within 2.2 of their impostors' green (8.3 before, sRGB means, all pixels)
and the pops the full-detail reference does not have fell 13 %; a still camera's frame-to-frame
change grew from 956 to 1003 pixels from the air and 398 to 474 at the green (3 codes); the
map check's foliage moved by ꟻLIP means up to 0.017 and was accepted.

**The shells' tone refitted (2026-10-29, #363).** The owner, walking in at noon: "still a lot of
popping (I believed imposters with not the same colours being replaced)". With the cards' leaves
kept, the 2026-10-25 fit (on green-only means, which left out the dark gaps the cards show) had
the shells 3–36 % brighter than the cards in linear light over every pixel of five patches of the
air view (the far woods, the near woods, the river's trees; 9:00, noon and 15:00), the impostors
0.84–1.23 of the cards. `CROWN_SHELL_TONE` 0.80, 0.74, 0.95 → 0.62, 0.58, 0.82: the shells now
within 0–12 % at 9:00, 3–6 % at 15:00, 11–18 % at noon, where the cards' crowns keep more inner
shade than lumps can. The map check's air view moved by ꟻLIP 0.025, the street views under 0.007,
accepted.

**The impostor's fade beside the ladder's crossing (2026-10-29, #363).** A tone-step detector on
the noon walk-in (16-pixel tiles whose mean steps in one frame while the frames around move
smoothly) still found a tree jumping from a soft crown to its shells in one frame. Every one of the
village's 81 tree models crosses from its cards to its shells under 1.67 times the impostors' 40 px
(13–60 px, the lumps few across), so the two bands overlapped, and the culls follow one crossing at
a time: the impostor, held back until the shells were whole, came in 19–100 % at once where the
cards' crossing ended (all of it for the 23 smallest, whose shells come in at 13–29 px). Now the fade runs
on its own band beside the crossing (`ladder_fade_after`): the cull measures its share once per
instance and packs it into the instance's ladder value; the cards and shells give up that share of
their pixels by a second noise (`ladder_fade_noise`, interleaved gradient noise), which the
impostor keeps; an instance its impostor would take whole draws nothing (a safeguard: with equal
bands and the hold below, a fade never ends inside the crossing into it). A step's fade is also held
to the size of the crossing into it (`meshlet::step_after`, the impostors' per instance): at 40 px
the shrubs by the main street went to their impostors, softer than their cards (less leaf
detail, the same mean colour; the map check's main view ꟻLIP 0.0074, 0.0032 held). Central steps
of 3 sRGB levels or more on the walk in: 24 → 5, of 5 or more: 5 → 0. Cost (`docs/PROFILE.md`): +0.02 ms from the
air, +0.07 ms on the main street. Moving each fade below the end of its crossing instead (the
shells drawn farther) lengthened the meshlet pass from the air by 0.24 ms.

*(research: forests-and-grass.md, vegetation-materials.md, flora.md; D-013, D-016, D-029, D-036,
D-053, D-063; issues #297, #300, #301, #302, #303)*

## D-076 — Navigation and traversal: a walk field with passage links first, Recast's polygons later ✅ (proposed 2026-10-19, #317, #316; the owner's go the same day, the player's crossing automatic)

**The need** (#317, #316, found by an out-of-tree character test): pursuers walk straight at their
target and slide along walls, never round a house or a fence. The walker does not know windows or
fences, so the scene finds the nearest window edge by distance and carries the body by script.
`game-ai-ecosystems.md` §2 (2026-09-26) chose Recast's pipeline on tiles with Polyanya and ORCA,
but no decision took it, and it had no patent section. `research/navigation-traversal.md` adds
one and the first step.

**Proposed** (`navigation-traversal.md` §6):
- **#317 first: a 2.5D walk field.** It is `walk.rs`'s rasterisation and clearance, as walkable
  spans per column at 0.10 m in 25.6 m tiles. It is built from the same prisms the capsule sweep
  uses (walls, partitions, furniture, flights, garden walls, hedges, trunks), on demand round the
  agents, and rebuilt when a piece changes.
- **The search:** A* over the spans and the passages' links, with integer octile costs, ties by
  cell index (D-016) and a node budget. With no path, the agent goes to the reachable cell nearest
  the goal. Then string pulling, with a link's ends as fixed corners.
- **The check (#276):** every path is swept with the capsule against the same prisms; a hit is a
  field bug, counted.
- **Following:** steering to the next corner, a hand-over at a link, separation first, ORCA after
  (`dodgy`/RVO2's algorithm with a fixed neighbour order).
- **#316: passages as data.** A `Passage` (door, window, low fence, tall fence, gap) carries its
  span, width, sill, head, depth, drop and state. Windows come from the structures' openings,
  fences from the garden walls, gaps from the kit's breaks, all made with the pieces and never
  detected from geometry.
- **The crossing:** chosen by the passage's kind and the agent's archetype in data (climb through,
  vault, climb, jump), each with its clip, its root motion warped onto the passage (#311), its
  time (the link's cost) and an exit check that hands back to the controller.
- **Later:** Recast's polygons and Polyanya for open terrain, with the same link records.
- **In Rust:** `rerecast` (deterministic with `libm`), `polyanya` and `landmass` stay to read and
  test against; nothing is bound to C++.

**Patents** (§4; claim 1s read, expiries estimated). None was found on Recast-style generation,
tiled rebuilds, any-angle search, ORCA or flow fields. The live ones sit at the edges, and the
proposal keeps clear of each:
- **Techland, US 8,979,652 B1 and US 9,238,175 B2** (to about 2034): a hook picked by a distance
  score, the action chosen by its height relative to the player. Here the passage's kind fixes
  the crossing in data, and the player crosses on a key.
- **Tencent, US 11,679,331 B2** (to about 2040): a pursuer that leaps at a target out of reach.
  Here jumps are links, or the agent waits at the nearest reachable cell.
- **Square Enix, US 11,406,898, US 11,241,621 and US 11,148,054** (to about 2040): a
  rooms-then-cells search. Here the search is one level until a measured need, then ask.
- **Disney (now Adeia), US 8,810,566** (to 2033-01): one mesh shared by several radii. Here
  clearance is kept per cell, or one mesh per radius class later.
- **Amazon, US 10,792,568:** bottleneck radii sent to clients. Here the sweep stays a test.

**Questions for the owner** (the recommendation in brackets):
1. Cells: 0.10 m in 25.6 m tiles, or `walk.rs`'s 0.05 m? [0.10 m: a 0.9 m door keeps three free
   cells for a 0.3 m walker.]
2. One walker for the field and the controller: the sweep's (step 0.35 m, top 1.95 m) or the
   physics controller's (step 0.4 m, 1.8 m)? [One record: the controller's step, the sweep's top.]
3. Who crosses what? [Climbers: windows and low fences; tall fences for athletic archetypes;
   wheelchair users: doors and ramps.]
4. The player's crossing: on a context key, or automatic on walking into a low fence? [The key;
   ask again before an automatic vault (Techland).]
5. Windows' states (shut, open, broken, barred) switching their links? [Yes; a shut window is a
   link only for archetypes that break glass.]
6. Long routes across the village: one level with a node budget, or a hierarchy? [One level until
   a measured need, then ask (Square Enix, Take-Two).]
7. When do Recast's polygons and Polyanya take over outdoors? [With the island's terrain; the
   field stays for buildings and streets.]
8. A lawyer's reading of Techland's two claims? [Before a public release, not before the
   prototype.]

**Decided** (the owner, 2026-10-19: "I will follow recommendations", one answer changed, and
"for your questions we will have to do some tests to see what works or not, but go with your
best guess or data"):
- **Questions 1–3 and 5–8 as recommended,** each open to the tests' numbers.
- **Question 4: the player crosses automatically,** walking into a low fence or a window ("for
  now"). It is kept clear of Techland's two claims by their two elements:
  - the passage is the one the player's movement line meets, never one picked from those near by
    a distance score;
  - the crossing is the passage's kind's in data, never chosen from its height relative to the
    body.
  The lawyer's reading (question 8) stays before a public release.
- **Space games ahead** ("nav needs to be advanced, but we can start simple"):
  - the passage and link records and the path query do not depend on the walk field, so other
    representations can serve the same queries later: Recast's polygons outdoors, a sparse voxel
    octree for flight and zero gravity (`game-ai-ecosystems.md` §2, Brewer's Warframe octree),
    a ship's interior in its own moving frame;
  - a field is built in the frame of what it stands on (a ship's deck, not the world), as the
    character controller moves with its ground.

**Step 1 built** (the same day, `forge-nav`):
- **The field:** `Field::build` from a tile's solid triangles (0.10 m columns, the person's step
  0.4 m and height 1.95 m, ground up to 45°).
- **Links and the search:** `Link` records, `Field::path` (A* with integer octile costs, the
  reachable place nearest the goal when there is no path), the path pulled straight, `reach` and
  `place_near` (a path's end in a bed takes the nearest place a person fits, within 1 m).
- **Checked on the village** (city-plan's `forge_nav_walks_from_the_street_into_every_room`):
  - the walker's pieces of 24 houses (walls open at their front doors, partitions, flights,
    floors, furniture) and the ground;
  - from 1.5 m outside each front door, 152 of 154 rooms are reached, 96 of them upstairs;
  - 38 ms a house's field (a tile of 0.10 m columns over the house and 4 m round it), 0.9 ms a
    path.
- **The two rooms not reached** are upstairs in one house. Its ground flight starts in a pocket
  about 0.6 m deep, fenced by its rail, that a walker 0.6 m across turns into with centimetres to
  spare; the field's clearance, a lower bound, counts it shut.
- **Cell size:** the 0.05 m columns reached fewer (144) at four times the cost; the clearance's
  half-cell bound decides more than the cell.
- **Left:** steering and avoidance, tiles built round the agents, and the town's pursuers as the
  first user.

**Passages built** (#316, the same day, `forge_nav::Passage`):
- **The record:** a passage carries its kind (door, window, low fence, tall fence, gap), its span
  on the wall's middle line, the way across, its sill and head, the wall's depth, the drop and its
  state (open, shut, broken, barred).
- **The crossing:** fixed by the kind (`CrossingKind`: walk, climb through, vault, climb, jump).
- **`Passages`:** an index by 8 m cells, `near`, `links` (an archetype's rule gives each link's
  cost and kind) and `met_by`, the passage a movement crosses first, which a player takes
  automatically.
- **The village** (`village_passages`): its houses' ground-floor windows and doors, and its garden
  walls as low or tall fences by their height, all made with the pieces.
- **Checked** (`a_shut_door_is_passed_by_a_window_for_climbers`):
  - with every ground-floor door of 8 houses shut, a walker no longer gets in from the street, and
    a climber does, all 8 through a window's link;
  - walking straight at a window from the street meets that window.

**Crossings built** (#316, the same day, `forge_nav::CrossingMotion`):
- **The motion:** three targets in the passage's frame:
  - the entry: where the feet are;
  - the top: over the middle line at the sill plus a lift;
  - the exit: past the far face by the body's radius and 5 cm, on the far floor.
- **Without a clip:** a curve. Before the top it rises early and goes ahead late; after the top
  it goes ahead early and comes down late. The top's time keeps the speed ahead even.
- **With a clip:** its root path (`RootPath`, sampled from the clip's root motion) is warped onto
  the targets. Each part is scaled per axis of the crossing's frame: Witkin and Popović's motion
  warping (1995), which the live traversal patents do not claim.
- **Spec:** `CrossingSpec` holds the seconds and the lift per kind. A person's defaults:
  - climbing through, 1.4 s;
  - a vault, 0.8 s with 15 cm;
  - a climb, 1.8 s;
  - a jump, 0.7 s with 40 cm.
- **The walker** (`city_blocks::walker`):
  - `cross` carries the body kinematically (Jolt's `place_character`).
  - At the end it checks the room at the exit (`character_fits`, Jolt's `CheckCollision`). The
    body stays there if it fits, and goes back to the entry if not.
  - `advance_across` crosses automatically, as the owner chose:
    - the passage is the one `meets` finds: the line 0.7 m ahead of the feet along the walk;
    - the walk must be at least 0.5 m/s and within about 45° of square;
    - the host's rule gives a crossing by kind and state, never by the body's height;
    - the crossing starts only where the exit has room.
- **The village:** its first-person walker vaults garden walls up to 1.1 m and climbs through
  the open windows of the houses with insides. Tall walls and doors are left to climbers and to
  walking.
- **Checked:**
  - forge-nav: both faces cleared, the way back over a drop, a clip warped onto a wall 0.25 m
    higher than the one it was made for;
  - the walker: a low wall vaulted, a shut window and a wall with no room behind it not
    crossed, a crate dropped at the exit mid-crossing sending the body back;
  - the map check's third walk, over the longest low garden wall on a street, shot at the
    vault's top; the other twelve images are unchanged.

**Following and avoidance built** (#317, the same day, `forge_nav::{Follow, Crowd}`):
- **Following:** an agent heads for its path's next corner and passes it within 0.25 m. It
  slows within 1 m of where it must stop: the path's end, or a link's start, where it waits for
  the game to cross (`at_link`, `crossed`).
- **Avoidance:** ORCA, written from RVO2's algorithm.
  - **Neighbours:** within 5 m on the same floor, the nearest 10, by a grid, ordered by distance
    then index.
  - **Constraints:** kept clear 2 s ahead.
  - **Order:** every velocity is chosen from the step's state.
  - **Deterministic additions:**
    - a nudge of 1 mm/s from a hash of the agent and the step, against perfectly symmetric
      meetings;
    - a turn to the right when blocked, up to 45° in proportion to the speed lost: people keep
      to one side, and a jam turns as one.
- **The ground:** a move the field does not walk straight is slid along x or z, or not made.
- **Checked:**
  - two people head-on;
  - twelve swapping across a 16 m circle, the same to the bit on a second run;
  - two groups of ten crossing;
  - sixteen pursuers closing on one point without overlapping;
  - a wall slid along;
  - two people through a 2 m opening from either side.
- **Cost:** a step of 1,000 people takes 0.83 ms on one thread.
- **Limits of this first step:**
  - Twenty-four swapping across the same circle pack into a ring that touches all round, which
    ORCA cannot set turning.
  - Walls are not among ORCA's constraints, only the field's slide.
- **Doors, the same day:** two people meeting in a 0.9 m door stood still, one wedged against
  the jamb. Three rules fixed it:
  - **Giving way:** two agents heading opposite ways, blocked face to face for 3 s, make the one
    with the higher index back off for 1.5 s. A shorter wait broke up the ring's unwinding.
  - **Back to the line near walls:** within 1 m of the walkable's edge, an agent heads 0.7 m
    along its path's leg, back to the line pulled clear of the walls. In the open it may drift
    aside, and pulling it back there jammed the circle.
  - **No turn near walls:** keeping right next to a wall only drives the agent into it.
- **Learned on the way:**
  - The first version's circle unwound only because every agent read as blocked on its first
    step and turned at once. The circle test now starts each person up to 10 cm off the
    circle, as people stand.
  - A "facing" weight read from the nearest 10 neighbours missed the people across a ring of
    12, so it was taken out.
  - Pursuers on a target's rim jostle within 2.5 m.

**Field tiles built** (#317, the same day, `forge_nav::Tiles`):
- **The tiles:** square, 256 columns of 0.10 m (25.6 m). Each tile's field covers its own
  columns and an 8-column margin, so its clearance at the edge sees what stands beyond.
- **Building:** the host builds the tiles round its agents (`round`) from the solids over a
  tile's `bounds`, rebuilds one alone when what stands on it changes, and drops those no agent
  needs. It may build a tile's field on any thread (`Field::build(tiles.desc(t), tiles.walker(),
  ..)`) and hand it over (`insert`, which gives back a field built for another tile or walker),
  as an out-of-tree town asked: it had cloned all its tiles to build a batch off the main thread.
- **A place** belongs to the tile owning its column. A step into a margin goes on in the owning
  tile, at the same point on the same floor. A tile not built is not walked, and a path towards
  it ends at the reachable place nearest the goal.
- **The search** is the field's, over (tile, place) nodes, on the tiles' common grid, with the
  links. `walkable_line` and the pulling cross tiles too.
- **Checked:**
  - a path round a wall's end across two tiles is 46.98 m, against 47.03 m on one field over
    both;
  - a tile not built stops the path at the edge of the one built, and building it opens the
    way;
  - two tiles built in 34 ms, the path in 8.5 ms over 76,420 places (dense arrays per tile,
    made when the search first reaches a tile; 25 ms with hash maps).
- **Crowds on tiles, the same day:** `Crowd::step_tiled` moves each agent on the field of the
  tile under its feet (`Tiles::field_at`, which `Follow::preferred` takes too). Two people
  crossing a tile border past a wall's end, either way, arrive.
- **A known case, left open:** rounding a wall's end tightly, one person passed two corners
  within 0.25 m, cut the corner, and stood wedged against the end, every move along its next leg
  refused. Two fixes were tried and taken back, because each broke the pursuers at the door
  (2, then 1 of 4 in):
  - reaching such a corner within 3–10 cm when the way on was not straight;
  - heading for a leg's start while short of it.

  **Fixed on 2026-10-22:** within reach, a follower passes a corner only where the way on to
  the next one is a straight walkable line from where it stands; otherwise it walks on to the
  corner (3 cm) or until it stops coming a centimetre nearer for 30 calls, half a second. The
  stall escape is what the door needed: there the pursuers queue, stop coming nearer, and pass
  as before. Both people now round the wall's end tightly (`a_crowd_rounds_a_walls_end_tightly`,
  from z = 10). The pursuers still all get in: 4 of 4 by the door, least gap −0.018 m (−0.028
  before), and 4 of 4 through the windows, −0.015 m (−0.000). A corner where a link starts keeps
  the old reach: its way on is the link, never walkable on the field, and creeping to 3 cm at the
  slowing speed held each crossing back about 1.5 s (found in review).

**The proof, the same day** (city-plan's `pursuers_get_into_a_house_by_its_door_or_through_its_windows`):
- **The setup:** four pursuers on the street, 3 m before the first house with insides, go for
  its hall on the house's field. They follow their paths, keep clear of one another, and cross
  each window link as a carried crossing.
- **Door open:** all four walk in.
- **Doors shut:** all four climb in through the windows, one at a time, each window's far side
  clear before the next. The least gap between two people walking is −6 mm.
- **Two things the test taught:**
  - A crossing's own exit, 0.35 m past the inner face, can fall where no place fits within a
    metre (furniture under the window). An agent therefore lands where its path goes on:
    `Follow::link_end` and `CrossingMotion::landing`.
  - Four crossings at once landed on one point. A passage is taken one at a time.

*(research: navigation-traversal.md, game-ai-ecosystems.md §2 and §7, animation.md §5; D-012,
D-016, D-062; issues #90, #276, #311, #316, #317)*

## D-077 — Streamed cells: a streamer driving the game's cell source, the far cells by a proxy each ✅ (proposed 2026-10-22, #338 step 4; the owner's go the same day, B: "I will follow the recommendations")

#338's last step: the town's map (78×63 cells, 9,547 buildings) cannot be held whole, so the
engine adds and removes its cells around the camera. Step 3 built every part a cell needs: the
dynamic scene's editor (meshes and instances), the probes' region sets, the glass's pane sets,
the lamps by stable ids, the pool of movers, and a cell's record (`CellSystems`) landing all of
it on one frame. D-037 fixed the plan: a clipmap of cells with hysteresis (`forge_world::
StreamPlan`, `Residency`, unit-tested).

**The streamer** (`forge_render::stream`, what D-037 already covers; built first):
- **The game gives a cell source:** a trait whose `load(cell, editor)` adds a cell's meshes and
  instances through the scene's editor and returns its sets (`CellSets`), on the streamer's
  worker. It reads the game's own data and cache (cooked meshes with their pages on disk).
- **The worker** holds the editor and the residency. It takes the viewer's position from the
  frames, makes the plan, and loads the nearest and finest cells first. Several cells
  go into one edit, since each edit rebuilds the editor's top-level structure off the frame.
  An unload removes the cell's instances and meshes in an edit.
- **The frames** take each finished edit with its cells' sets in one update (`apply` and
  `CellSystems::add` or `remove`), so a cell comes and goes on one frame. A frame budget caps
  the edits taken a frame.
- **Not here:** the far cells' look (below), and a cell's own state (doors open, things moved),
  which the game keeps and gives back when the cell loads again.

**The far cells' proxies: the open question.** A cell beyond the load reach shows nothing
without one. Three ways:

| | How | For | Against |
|---|---|---|---|
| **A. D-037's coarse levels** | Each coarser level's cells carry their own content, drawn until all their fine children are resident. | The plan as written: one proxy for a wide area, few instances far away. | While only some children are resident, the proxy and the fine cells overlap (z-fighting, doubled shadows), unless the proxy is cut per child, which is B. |
| **B. A proxy per cell** (recommended) | Two reaches per cell: within the near one its full content, within the far one its proxy, one merged mesh of the cell. The swap happens in one edit, so the proxy is hidden on the frame the content shows: never both, never neither. | No overlap rule. The proxy's DAG and the page streaming bring its detail down with distance for free. The game or an engine helper makes it from the cell's own meshes, cooked once into the derived cache. | One instance per far cell: 4,914 cells is many instances at the map's full extent, which a coarse level of B's proxies could merge later. |
| **C. Merged and simplified per group of cells** (Unreal's HLOD) | Groups' meshes merged and simplified offline. | The fewest triangles and instances far away. | The most tooling (offline merging and simplification per group), and the same overlap as A when a group is partly loaded. |

**Recommended: B for the town now** (near: the cells within about 0.5 km, full; far: proxies to
about 2 km; beyond: nothing, under the fog of the town's own look). D-037's coarse levels
stay for content of their own scale (terrain, the sky's distant ranges). C can come later
through B's proxies.

**Patents** (a first search, 2026-10-22: web searches and Google Patents' pages of the three
nearest hits; a full search before C is built):
- **US 10,825,236 B1**, "Composite mesh LOD construction":
  - Arvizio Inc., US, active, expected to expire 2039-03-12.
  - Claim 1 sorts a model's objects into classes processed alike, builds an LOD structure per class with a rendering strategy that depends on the viewer's location *and gaze direction*, then merges them into one structure looked up by location and gaze.
  - B (a merged mesh per cell, chosen by its size on screen) has no classes and no gaze-dependent strategy, so it does not match.
- **US 12,589,303 B2**, LOD assets for a videogame session:
  - Activision Publishing, US, granted 2026-03-31, expected to expire 2044-05-16.
  - Claim 1 builds a point cloud with normals around part of a landscape, generates a mesh from it, and simplifies that mesh into proxy meshes swapped at run time.
  - B merges the cell's own meshes and simplifies them by their cluster DAG, with no point cloud. C must not remesh through a point cloud (some HLOD tools do).
- **US 2014/0354626 A1** (Google, block-based LOD of buildings by height in a quadtree): abandoned.

**B built** (2026-10-22, the owner's go the same day):
- **The plan:** `StreamPolicy::proxy_rings` (0, none, by default). A level's cells within its
  `rings` are wanted as their content, those beyond and within `proxy_rings` as their proxy
  (`forge_world::Detail`). `Residency` swaps a proxy to its content as soon as it is wanted, and
  content to its proxy (or a proxy to nothing) past the hysteresis, as it unloads.
- **The streamer:** `CellSource::load_proxy` adds a cell's proxy (nothing by default). A
  swapping cell's old detail is taken out in the edit that brings the new one, and its records
  are swapped in the same update.
- **The merge:** `forge_geom::proxy::merge`, the cell's placed meshes as one, its instance's
  row the lowest of the parts', each triangle's section the distance to its own row. The game
  chooses the parts and cooks the result once (`cache::cook_cached`, keyed by the cell's
  content).
- **Checked** in the lab (`FORGE_DYNAMIC_SCENE=cells`, `docs/demos/physics-lab.md`).
*(research: large-worlds.md §5; D-037; issue #338)*

## D-078 — Rooms lit by probe grids of their own, the cascades for the outside and far rooms ✅ (accepted 2026-10-23, #353)

**The question:** a room surface takes its light from the finest camera-centred cascade that
covers it. The coarser cascades light rooms darker, so a far wall brightens as the finest
cascade reaches it. The owner saw it in a game's 20 m hall: "the wall lights up if you move
forward". ebbd2075 spread the change over 4 m and took the coarser cascades' bounce loss away
(#353). What remains is the near-wall probes. In a 2 m cascade, the layer nearest an outer wall
often stands in the wall, a window's recess or just outside. The rooms' masks (D-068) rightly
leave those probes out, so the wall reads probes up to 2 m back. On the game's wall, without the
masks the 2 m light matches the 1 m one: 132 against 131, and 107 with them. The game runs at
1.5 m spacing meanwhile, where its hall stays in the finest cascade.

**Research:** `research/probe-levels-indoors.md` (2026-10-23, the owner's go for a patent-free
answer).

| | How | For | Against |
|---|---|---|---|
| **A. A probe grid per room** (recommended) | Each room volume the generator gives (D-068) gets a small world-anchored grid, its outer layer 0.3–0.4 m in from the walls, floor and ceiling. Its surfaces and the probes' hits in it read that grid alone: no masks, no cascade loop, a linear blend. Doorways ramp between two rooms' grids. The cascades' probes inside such rooms go inactive. Near houses only (the ones whose insides are drawn), within a radius with hysteresis, each room cross-fading in over about 0.5 s. | No step: the grid doesn't move with the camera. Every wall has probes in front of it. Prior art: Source's per-leaf ambient cubes, Blender's room-aligned grids, DDGI's dense volumes. Moves further from Square Enix's claim 37 (one region number per grid, chosen up front). | A new lookup path and a pool of room probes in the atlases. Odd-shaped rooms need several boxes. Open-plan spaces without volumes stay on the cascades. Cost estimated at 0.05–0.3 ms for 1,000–2,000 room probes, less the in-room cascade probes it retires; to be measured. |
| **B. The cascades as they are, tuned** | The finest cascade wider (`ProbeParams::finest_counts`, built in d22d3779: 32 across, +0.21–0.32 ms, reach 14.5 m), and rooms blended linearly. | Small, mostly built. | The step stays, only farther. Linear blending brightens rooms 2–4 levels and narrows the gap by ~1.5 (measured). |
| **C. Companion probes by outer walls** | In the coarser cascades, a probe standing in a wall, a recess or outside, with a room within half a cell, gets a companion in the room 0.3 m from the wall. The lookup takes the companion when the probe isn't the room's. | Fixes the near-wall cause in the coarse cascades only, at the perimeter's cost. | The step from the cascades' other differences stays. Region-aware relocation, its nearest try, gave +4 in one room and −6 in another. |

**Recommendation:** A, built in steps, each measured with #353's drift test and the map check:
1. The pool and the grids for the camera's house, behind a switch.
2. The lookup and the hits.
3. Retiring the cascades' in-room probes.
4. The radius and the cross-fade, then on by default where the scene gives rooms.

B's linear blend could come first as a look change for the owner to judge, but it doesn't
remove the step.

**A patent the research found, for the owner:** NVIDIA's US 11,790,594 B2 (active to
2041-10-17).
- **Its claim 1:** rank the locations holding irradiance caches, cast rays from the higher-ranked
  ones, and render. Its dependent claims rank by distance to the camera and by frames since the
  last update.
- **What comes near it here:** Forge's cadence (#103) updates settled probes every `cadence`
  frames, on a fixed turn, not by a ranking. The village and the town run cadence 1; the city's
  default is 2.
- **Under A:** every room probe updates every frame with the same rays, so nothing is ranked.
- **Not decided here:** whether a fixed round robin reads on the claims is a question for
  counsel.

**The owner's answers (2026-10-23):** yes to A. On the patent: continue as is; later, or for a hard
problem, try the patented best in a private lab on this machine (never on GitHub), then find a
technique with similar results that does not infringe.

**Built (2026-10-23, all four steps; on wherever the scene gives rooms, `--no-probe-rooms` off):**
- **The grids** (`probes/rooms.rs`): a box over each room volume's outline, along the edge that
  gives the smallest box, probes at most 1 m apart across and up (three layers in a room 2.7 m
  high), the outer ones 0.35 m in. A pool after the cascades' rows of the atlases (4,096 probes)
  holds the grids of the rooms within 20 m, the nearest first; a full pool lets its farthest grid
  go for a room 5 m nearer, so in a street of houses the pool sets the reach (33 rooms there).
  A new grid takes its 8 young updates, then fades in over 30 frames; one 10 m past the radius
  fades out.
- **The lookup** (`room_irradiance` in `probes.slang`): the eight probes around the point, facing,
  Chebyshev and the crush as in a cascade, blended linearly. A room's surfaces and the probes' hits
  in it read its grid alone; across a doorway each side its own (or the cascades' for a side
  without one); a point in a house's body (a corner, a window's frame) the grid within 0.6 m.
- **The cascades' probes** standing in a room whose grid is fully in retire (state 3): not traced,
  blended or read. They come back young when it fades out. The village's room view: 1,739 retired
  (1,391 of the 1 m cascade's) for 3,232 grid probes in 26 rooms.
- **Two errors found on the way:**
  - Two layers 2 m apart left a bright V over a table: the lower probes stood under it.
  - Rays that met a house's body outside every grid's box took every probe's light. With the
    room's cascade probes retired, only the bright outside ones were left, and the room came out
    6 levels too bright. Taking the grid within 0.6 m fixed it.
  - The first lookup called the cascades' from five places, each an inlined copy, and a start
    after a resolve edit took 267 s. One call (the grids' sides summed in a loop, the cascades
    for what they leave) brought it to 13.4 s with #348's other changes (`PROFILE.md`).

**Measured (2026-10-23, the village at 18:00):**
- **#353's drift test** (the room settled, then the cascades drifted 14 m off): the cascades alone
  change by 8.5 to 23 sRGB levels across six patches; with the grids, by 0.5 at most.
- **Against probes twice as dense** (cascades 0.5 m apart, grids 0.75 m): 1 m grids came within
  2.5 levels in the room and the hall, 1.5 m grids up to 7 too bright, the 1 m cascade up to 8.5
  too dark.
- **Serial frame** (`FORGE_ASYNC=0`, three runs each): the room +0.20 ms, the hall +0.12, the
  house from the street +0.26, the main street +0.02. The probe passes run on the async queue.
- **The map check:** the outdoor views unchanged (ꟻLIP ≤ 0.002), the insides changed
  (0.03 to 0.09), accepted.

**Rooms beyond the grids' reach (2026-10-24, #354, #356).** The owner, walking towards the
houses: "the inside of the rooms look wrong at some transition points". Beyond the room-sized
cascade a room took a coarser cascade's probes standing in it (dark, #353), and lit up when its
grid came within reach. Since then, wherever the scene has grids, every such room takes the far
room's share of the open sky (`far_room_sky`), set against the grids: the village's 0.2 keeps the
windows within 0.02 EV of grid-lit rooms in the afternoon and under a low sun (0.06 was 0.05 EV
dark). The grids take the sun's bounce, which the share cannot follow through the day: #356.
The grids' probes also stay on their lattice: one standing in a landing's guard was moved out on
the nearer side, the stairwell's, and lit the landing's floor along the guard's foot in spots a
metre apart (the owner's sighting); one inside a solid is now inactive, its neighbours lighting
round it (the map check's insides within FLIP 0.04, the outside views unchanged).

**The far rooms' daylight (2026-10-29, #356, on the owner's go).** Against the grids
carried out to 150 m (`--probe-rooms 150`, the reference), the fixed share read the far rooms'
windows on #354's dolly +0.08, +0.19, +0.01, −0.14 and 0.00 EV at 09:36, noon, 13:48, 15:00 and
16:12 (`--time-of-day` 0.3 to 0.85): too bright at noon, dark in mid-afternoon. Each far
room with windows now takes a share of its own (`ProbeParams::far_room_daylight`, the village's
look):
- **The share:** K × Σ over its windows of area × (the sky's irradiance on the window's outward
  normal + the sun's on it), over the room's surface × the open sky's irradiance from above. The
  windows are the room's recesses (`probes/far_rooms.rs`): the normal along the recess's short
  axis, away from the middle of the room's volume it opens from, the area its long edge by its
  height; a room of several volumes sums them less the faces they share. K stands for the
  light's bounces in the room, about 1 / (1 − ρ) for walls of albedo ρ.
- **Made each frame on the GPU** ("gi/far rooms", the async compute queue, 0.008 ms in the
  village) from the sky's SH light and the sun, into a table hashed by region that the far
  branch of the probes' lookup reads where it read the fixed share; a room without windows, or
  one the table has no room for, keeps `far_room_sky`.
- **Calibrated:** K = 1 read 0.59–0.75 EV dark at every hour, 1.57 still 0.34–0.52 (about 40 % of
  those pixels' light is not the room's share: the frames, the glass, the sun through the
  window). The village's K = 2.7 reads +0.04, +0.01, −0.07, −0.15 and +0.01 EV: noon's excess
  gone, the day's spread 0.33 → 0.18 EV; the deficit at 15:00 stays as it was.
- **A bug on the way:** the buffers were made only for scenes giving rooms by sets
  (`reserve_regions`), not for the village's whole set (`set_regions`), so the first round
  measured the fixed share twice; a GPU test now gives rooms both ways.

**The far rooms' lamps (2026-10-29, #363).** The owner, at midnight: "when getting closer the
windows change too much". A lit room's light round its walls came from its grid's probes alone,
and the probes see only the frame's 64 nearest lamps: past the grids (20 m) a lit room kept its
lamp's direct light but lost its bounce, its floor and furniture going dark under a bright ceiling,
and past the lamps' cut (50 m) the window showed the pane's glow over a room lit by nothing. One
window seen straight on at midnight (fixed exposure, the window's mean in the capture) stepped
from 0.79 to 0.53 at the grids' edge and stood at 0.37 at 70 m, against the grids carried out to
150 m with every lamp lit (0.68 to 0.97).
- **The light round its walls:** each far room with windows also takes its lamps that are on
  (`lamps::room_lumens`, each lamp's room its first region), their lumens over its surfaces times
  `bounce`, and times `unlit` more for the share of each the frame leaves unlit (its direct light,
  past the frame's lamps): `ProbeParams::far_room_lamps`, `[bounce, unlit]`. The far rooms' pass
  writes it into the table beside the share, three halves in lux, and the probes' lookup adds it
  to the far room's light in the resolve's units (`far_lamp_scale`, 1 over the sun's illuminance:
  in those units a night's reference of a thousandth of a lux, after moonset, put a lit room past a
  half's range). The rooms' sums follow the frame's lamps (`Probes::set_room_lamps`); when they
  change only the rooms' records are written again, not the table.
- **Calibrated** against that reference at 20 to 90 m: bounce 14 read 0.02–0.03 dark at 20 to
  45 m, 20 as bright; the village's 18 gives 0.688 and 0.844 at 20 and 45 m (0.682, 0.847). Past
  50 m the pane's glow was not what was missing (twice `ROOM_GLOW` moved the window 0.01): unlit 14
  gives about 0.92 and 0.96 at 70 and 90 m (0.937, 0.966). The factors are far above a room's
  1 / (1 − ρ): the window shows the walls near it, which the lamp lights least directly, and the
  captures' tone curve compresses the steps it is set by.
- **Seen:** on #354's dolly at midnight the room under the lit balcony now glows through its door
  as the grids light it; the map check (15:00) moved by ꟻLIP up to 0.009, a far room through a
  window, accepted.

**The furniture out of the probes' rays (2026-10-29, #363).** The owner's walk named three
rooms' light ("lighting issue", "quite a lot of lighting issues"). In a bedroom the floor took its
light in sharp curved patches, cool against warm, and the walls an orange cast. The patches were
the grids' visibility test: a bed or a wardrobe in a probe's way cut its view of the floor, and
the floor beyond took none of that probe; a grid twice as dense shrank them, none took them out.
The cast was the orange furniture's bounce. City-plan's furniture (the kit's home pieces) is now
out of the probes' rays (`Visibility { probes: false, .. }`): drawn, shadowed and mirrored as
before. The patches go (the bedroom's floor even), and in the map check's room the back wall and
the floor read about five levels brighter and lose the orange (rgb 95, 87, 58 → 92, 95, 67), the
grid's density no longer mattering (0.5 m within a level of 1 m). The map check's insides moved
by ꟻLIP up to 0.134 (the room) and 0.053 (the hall), accepted; `--furniture-in-probes` keeps
them in to compare.

**Halls (2026-11-04).** In the private district (D-083 step 4) the bar's two floors are a region
each, about 700 m² and 3.8 m high. 1 m apart the ground floor's four boxes asked 4,625 probes,
more than the whole pool (4,320 there). The pool took three of its boxes and stopped at the
fourth, so the floor above never got a grid. Its points took the far rooms' share of the open
sky, and at 16:21 its railing, trusses and ceiling glared white over a dim bar. The owner: the top
half "seems to not fit with the bottom half". A region's grids now take at most
`RoomGridParams::region_probes` (1,024, a quarter of the default pool). A larger region's probes
stand farther apart, across and up alike, by the least step of 10 % that fits, three times the
spacing at most. The scale is worked out once per room set. The bar's ground floor takes 1.77 m
(about 1,008 probes) and the floor above gets its own grid, lit by its lamps and the neon below. The village's
largest room asks 528, so its 13 map-check shots are unchanged (ꟻLIP 0). On the district's
benchmark path the frame did not cost more: 17.21 against 17.30 ms, then 16.51 against 17.12, the
probes' rays 2.84 → 2.65 ms on average. `--probe-room-region` sets the cap (100000: none). A
room grid builds its distance maps with the rooms' wider lobe (#283) at any spacing: the choice
went by the spacing alone, and a hall's grid past 1.5 m took the cascades' narrow one (the bar's
GI view moved by ꟻLIP 0.05, the lit view 0.01).

*(research: probe-levels-indoors.md; D-068; issue #353)*

## D-079 — Marks where bodies strike: a pool of run-time decals on static surfaces ✅ (proposed 2026-10-24, accepted the same day, Phase 7)

**The need:** Phase 7's contact events now sound by material (`physics-lab`'s impacts, D-007's
sound layer). The roadmap's next step is their marks: where a body strikes a static surface hard
enough, a scuff on stone, a dent or scratch on painted metal, a dark print on soft ground, chosen by
the struck and striking materials and scaled by the impact. D-074 decided decals on static
surfaces from a world grid **built once**, "nothing on movers, no bullet holes" for now. Marks
added at run time change the "built once", so this needs the owner's word.

**What games do** (`research/impact-marks.md` §2–3): impact marks are ordinary projected decals
with a lifetime or a fade, capped by a count, the oldest dropped first (Unreal, Source's 2048,
s&box, Godot, HDRP). The mark is chosen by the struck surface's type (Source's game material,
Unreal's surface types) or a surface × surface matrix (CryEngine's material effects), a random
variant within a size range. Strength gates are a minimum speed, mass or impulse (Chaos) or an
energy (CryEngine's ½mv²).

**Proposed:**
1. **A pool beside the static decals:** 1 024 run-time marks by default (4 096 at most), 64-byte
   records like `GpuDecal`, in a ring; the spare words hold the birth time, the strength and the
   fade's start, so the shader fades them and fading uploads nothing.
2. **A second one-level grid of 4 m columns** for the marks, each column's list inline (at most 8,
   a ninth recycling that column's oldest), updated in place on the CPU when a mark is added
   (about 256 bytes uploaded a mark). Never a coarser level over it, never screen binning.
3. **From a contact to a mark,** on the game thread after the step, beside its sound: contacts
   with one fixed body, not a networked client's re-run ticks; the energy `½ μ v²` (μ the reduced
   mass `Impact::between` already uses); a table (struck surface × striker) → the mark's kind,
   with its least and full energies, sizes and variants; a near mark of the same kind strengthened
   rather than a new one; the box along the contact's normal, 3 to 5 cm deep, its roll and variant
   from a hash of the tick and the bodies (the same marks every run and in replays). At most 8
   marks a tick; a body marks again only 10 ticks later.
4. **The look:** textures made in code, faint and warm (the 2026-10-19 lesson: round dark stains
   read as shadows), a 0.15 s fade-in so a mark never appears in one frame under DLAA, a 2 s
   fade-out once the ring is 90 % full. Applied in the resolve like the static decals: the
   surface's motion vectors kept, probes and mirror rays seeing bare surfaces.
5. **Soft ground with a deform layer** gets a press (D-007's `deform`), not a mark.
6. **Then:** dents and scratches need the colour decals' normal and roughness D-074 planned; city
   meshes need the struck triangle's row in `Contact` (Jolt's per-triangle materials).

**Patents** (`research/impact-marks.md` §5): none found on decal pools, recycling, fading,
overlap rejection, material-pair tables or energy thresholds (published practice: Source SDK
2013, CryEngine 3, Unreal, Godot). The binning patents met require coarse-to-fine levels over a
field of view or image tiles: Activision's US 12,064,688 B2 (to 2042) and its pending continuation
US 2024/0408486 A1, Apple's US 9,183,664 B2 (to 2033), AMD's US 10,803,655 B2 (to 2036). One
world-space level does neither, hence "never a coarser level". Take-Two's US 11,534,688 B2 (to
2039, trail maps for soft terrain) bears on D-007's deform layer, not on these marks: claim 1
stores marks made away from the camera and writes them into a trail map when the camera comes
near. The deform layer's window drops what lies outside it (read 2026-10-24, §5.1); it must keep
doing so.

**For the owner:**
1. **Scope:** run-time marks on static surfaces, changing D-074's "built once"? Bullet holes and
   scorches later through the same pool?
2. **Lifetime:** kept until recycled (proposed), or a time limit as well?
3. **State:** presentation only (not saved, not in replay digests, each client its own;
   proposed), or simulation state?
4. **Normals and roughness** for colour decals first (for dents and scratches), or scuffs and
   prints only to begin with (proposed)?

**The owner's answer (2026-10-24):** the recommendations, all four: run-time marks on static
surfaces, kept until recycled, presentation only, scuffs and prints first. For bullet holes, when
they come through this pool, the owner prefers depth drawn with **parallax mapping** where it fits
(the owner's references: Counter-Strike: Global Offensive or Counter-Strike 2, and Red Dead
Redemption 2), before geometric chipping or real holes, or combined with them. That step gets its
own research note first: how those games draw holes, and the patents on parallax and relief
mapping of decals.

*(research: impact-marks.md, decals.md; D-074, D-007)*

## D-080 — A night with lamps keyed on its lamps ✅ (proposed 2026-10-24, #354; the owner, 2026-10-25: keep the moonlit key for now; the owner's go of 2026-10-31: step 1)

**The need:** the owner, on the village at night: the lit windows read "too strong … they are
house lights, not spotlights". The research (`research/home-lighting.md`) finds Forge's lamps and
faked windows close to real values (lit windows measure 1–8 cd/m² behind curtains; Forge's are
about 5). What makes them glare is the exposure: D-046's film night keys the scene on moonlight
(EV100 about −2), where anything over about 2 cd/m² lands in the top 15 % of the display. A curtained
window, a lamp-lit wall and a street lamp's pool are squeezed together near white. Photographers
meter lit buildings at EV 2, and cinematographers set practical lamps 2.5–4 stops over the key.

**Built so far (2026-10-24):** the bloom at night a quarter of the day's; and, opt-in,
`r.exposure.night_floor` (`AutoExposure::night_floor`, EV100; −6 by default, the automatic range's
floor: no change): at night the exposure takes no lower EV100 than the floor, by the night's share.
Local exposure's middle grey stays the moonlit key (the floor is kept out of `night_offset`), so it
lifts the fields the floor darkens. A first try keyed on the frame's brightest 3 % instead: in the
village's views those are mostly the lamp-lit paving, not the windows, and it changed nothing.
Measured at three night views (the owner's): at EV100 0, the windows show their rooms (the shelves,
the lamp's warm light) instead of white, the lamp-lit lawn and paving stay readable, the moonlit
facades and the sky go darker; at +1 the street round them is nearly black.

**Proposed:**
1. `r.exposure.night_floor 0` by default where a scene has lamps (the village, the city), after
   the owner's look at the A/B shots in the morning report.
2. Then, the research's other steps, each its own change: shades on the house lamps (two-lobe
   profiles, the shade glowing about 100 cd/m²); shutters and curtains per window chosen by the
   household (never by a texture-space floor pattern: Bosch's EP 3238179 B1, granted 2026-02-11, and
   US 10,380,790 B2); one or two lamps on per occupied home rather than every room; village
   lanterns at 1,000–1,500 lm on 4–5 m brackets (EN 13201 P4–P3), once the exposure no longer makes
   real values glare.

**For the owner:** keep the moonlit key (D-046 as is), or floor a night with lamps at EV100 0 (or
another value from the A/B)?

**The owner (2026-10-25):** keep the moonlit key for now (the A/B's left column); either looks
right, and the moonlight may be a bit too bright. `r.exposure.night_floor` stays built and off.

**Step 1 taken (2026-10-31, the owner: a lit window "looks like a nuclear plant powered bulb";
"I will follow your recommendations"):** a night with lamps (the scene gives lamps, and no
`--no-lamps`) floors its exposure at EV100 0 (`LIT_NIGHT_FLOOR`, `vars::register_vars`); a night
without keeps the moonlit key. In the village's junction view at midnight the meter went from
EV100 −2.17 to 0: the window's 25 cd/m² two stops less over, the moonlit facades darker, the lamp
pools readable. The other half of that window's look is #378's halo, still open (D-068's amendment). Steps 2
(shades, shutters and curtains, fewer lamps on, lanterns) wait.

*(research: home-lighting.md, night-sky.md, local-exposure.md; D-046, D-067)*

## D-081 — The first launch fits the machine, and the menu's graphics page keeps the user's choices ✅ (the owner's ask, 2026-10-25)

**The owner (2026-10-25):** "The first time it's launched the engine should adapt itself to
(resolutions and other technology) to the computer it's on like modern game engines do. And the
user could change some settings in the game parameters menu under graphics like modern game
engines do."

**Built (2026-10-25, on the audit's configuration spec, its user layer):**
- **The user's file,** `settings/user.cfg` (`FORGE_SETTINGS_DIR`): `name value` lines as
  `forge.cfg`'s, marked as the user's (`Source::User`), read at start before `forge.cfg` and
  `--set`, which override it, and never by a scripted run (`FORGE_CFG=none`, `--no-cfg`). The
  shell writes it again whenever a setting is set as the user's (`forge_core::settings`:
  `load_file_as`, `user_lines`, `take_user_changed`).
- **Quality presets** (`r.preset`: custom, low, medium, high, ultra; `forge_app::presets`): a
  preset is a patch over the renderer's shared names (how small DLSS and FSR render, the ambient
  occlusion, the mirror reflections, the local exposure, the detail error), applied as from where
  the preset came; a patched setting changed by hand puts the preset back to custom. `r.aa` is
  left as the device decided it (DLAA where DLSS runs, else FSR).
- **The display** (`app.window`: windowed or borderless fullscreen at the monitor's own size),
  followed each frame, and following Alt+Enter for the run.
- **The first launch:** with no user file and an interactive run, the shell reads the GPU's
  device-local memory, whether it runs DLSS and the monitor, picks the preset by the memory (the
  target's 16 GB and 12 GB ultra, 8 GB high, 5 GB medium, less low; unknown medium) and
  fullscreen, and writes the file with what it saw as its first lines.
- **The menu's graphics page** (Esc, Settings, Graphics): the preset, the display, the
  anti-aliasing, the upscalers' modes, the ambient occlusion, the mirror reflections, the local
  exposure, the bloom and the detail error, those the demo registered; every change in the menu
  is the user's and lands in the file.

**Checks:** unit tests of the presets (the memory's steps, the patches' names), the user file's
round trip and its rank under `forge.cfg` and `--set`, and the menu's graphics page. The look
is unchanged: a scripted run reads no user file, and `r.preset` is custom by default. Written
on a laptop whose Vulkan loader lists no NVIDIA GPU: the first launch and the menu are for the
owner's machine to try.

**Not done:** a resolution scale apart from the upscalers' modes; presets fitted by a measured
frame time rather than by memory; the menu's page for the sound and the keys.

**Amended (2026-10-31, D-092):** the user's file keeps every choice until Defaults removes it
(one `forge.cfg` or `--set` overrides, one the run does not register, one equal to the default:
an override used to drop the line at the next write), and a scripted run (a frame limit or a
capture) never reads or writes it, whatever `FORGE_CFG` says. The graphics page became a
settings screen of three pages.

## D-082 — AMD's FSR DLLs wherever the system has them (FSR 4 on AMD's cards), Forge's FSR 3.1 elsewhere ✅ (the owner's go, 2026-10-29, #361)

**The owner, the same morning:** "We should probably adopt the FSR dlls if available on the
system though": under `r.aa=fsr`, `auto` takes the DLLs on any GPU wherever the system has
them, and Forge's FSR 3.1 runs only where it hasn't (the choice below, amended).

**The owner (2026-10-29):** "And FSR 4 module after", on the proposal of
`research/fsr4-vulkan.md`: FSR 4 behind `r.aa=fsr` on AMD through the FSR SDK 2.4.0's signed
DLLs, the Slang FSR 3.1 port (D-070) kept everywhere else, built as a load-and-fall-back path
before there is an AMD card to test it on (#67).

**Built (2026-10-29):**
- **The binding** (`forge_gpu::ffx`, the `ffx` feature, on in the demos): the FFX API's C
  functions from `amd_fidelityfx_loader_vk.dll`, loaded at run time, their descriptions chained
  by `{type, next}` headers laid out as the SDK's headers lay them (`repr(C)`). `ffxQuery` lists
  the versions the DLLs offer for the device ("3.1.6" and "2.3.4" on the 5070 Ti and on the RDNA 2
  iGPU). The DLLs' own choice is the newest, a driver's ("FSR 4.1.3 *") when strictly newer;
  an override pins a version and opts out of the drivers' updates (AMD's `ffx-api.md`), so
  `FfxPick` gives one only where their choice is not FSR 4 and an FSR 4 is offered (the newest),
  and `Only(4)` makes no context without one. The version running is read back from the
  context.
- **The pass** (`forge_render::FfxUpscaler`): "temporal/FSR (AMD)", one graph pass declaring the
  jittered colour, its depth, the motion vectors, a 1 × 1 exposure of 1 (the frame is
  pre-exposed, as for DLSS), the reactive mask where the frame has one, and the output, written
  as storage. The DLLs record their own passes and barriers inside it and leave each image in
  the layout the graph gave it, as Streamline does for DLSS. RCAS follows `r.fsr.sharpen` and
  `r.fsr.sharpness`, as for Forge's FSR. A dispatch the DLLs refuse costs that frame's upscale,
  not the frame: the demo then drops their context and hands the frames to Forge's FSR 3.1.
- **The choice** (`--fsr-amd`, under `r.aa=fsr`): `auto`, the default, takes the DLLs on any
  GPU wherever the system has them: FSR 4 where they offer it, else their own choice (their FSR
  3.1.6 on the 5070 Ti and on the RDNA 2 iGPU). `on` does the same and warns where the system
  hasn't them; `off` never takes them. Wherever they are not taken, Forge's FSR 3.1 runs; the
  FSR captures and the validation run pin `off` to keep testing it. Until the owner's word
  above, `auto` loaded them on AMD's devices only and only for FSR 4.
- **Where they are looked for,** with the first FSR (at the start, or when the console first
  asks for `r.aa fsr`): `FORGE_FFX_DIR` when set, else beside the executable (a player's
  install), else the git-ignored `fsr-sdk/Kits/FidelityFX/signedbin` (the night's tree has it
  through a junction, as the other SDKs).
- **The device:** `VK_KHR_compute_shader_derivatives` (`computeDerivativeGroupLinear`) is
  enabled where present: the DLLs' shaders declare it, and the validation layer flagged it. So
  are, where present, what AMD's own framework enables for FSR 4's models:
  `storageBuffer16BitAccess`, `shaderIntegerDotProduct` (the INT8 models),
  `VK_KHR_cooperative_matrix` and `VK_EXT_shader_float8` (the RX 9000's FP8 model).
- **The DLLs** come from `FORGE_FFX_DIR`, else from beside the executable, else
  `fsr-sdk/Kits/FidelityFX/signedbin` (the SDK, git-ignored). Like Streamline's, they are never
  committed or published on GitHub: nothing of AMD's FSR SDK, its ML upscaler included, enters
  the repository (the owner, 2026-10-29; `CREDITS.md`). A shipped game bundles them (the owner,
  2026-10-30): the SDK's licence (2.4.0, `docs/license.md`) lets the signed DLLs go out free in
  binary form only, with AMD's copyright and permission notice in the game's documentation and
  no reverse engineering. The Vulkan ones are `amd_fidelityfx_loader_vk.dll`,
  `amd_fidelityfx_upscaler_vk.dll` and `amd_fidelityfx_framegeneration_vk.dll`; the denoiser's and
  the radiance cache's are DirectX 12 only. Streamline's
  signed DLLs and `nvngx_dlss.dll` ship the same way, never its development DLLs.
- **Forge's FSR 3.1 stays in the repository** (the owner, 2026-10-29: "follow your
  recommendations"). Its 21 shader headers (`shaders/third-party/fidelityfx/`, FidelityFX SDK
  1.1.4: FSR 3.1, FSR 1, SPD) are AMD's open source under MIT, which allows publishing and
  redistributing them, and they are the anti-aliasing of every player without DLSS or AMD's
  DLLs. The rule covers what may not be redistributed: the SDK's signed DLLs and FSR 4's model.

**Measured (2026-10-29, RCAS on in both):**
- **The 5070 Ti** (`--fsr-amd on`, the city's frame 120 at 1600 × 900): `temporal/FSR (AMD)`
  0.288 ms against Forge's nine passes' 0.317; against Forge's image, ꟻLIP mean 0.015 (p99
  0.055).
- **The RDNA 2 iGPU** (`FORGE_GPU=amd`): the DLLs offer no FSR 4 there; their 3.1.6 runs in
  14.2 ms against Forge's 18.7, without a validation message (what `auto` takes since the
  owner's word above; at first `auto` kept Forge's FSR 3.1 there).
- **Validation:** no VUID on either GPU. Inside the DLLs, the layer warns that some of their own
  storage images (`rw_spd_mip*`, `rw_current_luma`, 16-bit) are written through 32-bit format
  declarations: AMD's images and shaders, none of Forge's.

**Open:**
- **FSR 4 itself** waits for an RX 6000/7000/9000 card (#67): whether the features above are
  all it needs, and whether it takes inputs left in `GENERAL` (the graph's layout for a sampled
  image that is also a storage image), which its guide asks in `SHADER_READ_ONLY_OPTIMAL`.
- **Patents:** the SDK is MIT, which grants no patent licence; AMD's patents on its ML upscaling
  are to be looked up before a release relies on the DLLs (the patent rule).

*(research: fsr4-vulkan.md; D-070; issue #361)*

## D-083 — Authored scenes and a path-traced reference before more tuning; one theme at a time ✅ (the owner's go, 2026-10-29)

**The owner, 2026-10-29:** "we spend all our time solving the issues with the probes and night
and fake windows at night. We also end up with more issues at the end of the day than at the
start of it. Procedural stuff seems to be difficult as well. How would you improve our way of
working and make some actual progress?", then, on the answer below: "go, we will follow your
steps".

**What the numbers said (2026-10-29):**
- **The backlog grows:** in the weeks from 8 and 15 October, 86 and 83 issues were opened and 42
  and 48 closed; 107 are open.
- **Where the work goes:** of 572 commits since 15 October, 228 touch the probes, the far rooms,
  the windows, the lamps or the night.
- **Every batch reviewed after its push had real bugs** (the review agents, 2026-10-23 to 29).
- **The village's walls and lots** are about 3,000 lines with 25 tuning constants, most of them
  cases added after a sighting.

**Why:**
- **Approximations patched with approximations.** A room's light at night comes from the
  cascades, its room grid, the far room's share and the far lamps; each fix moved the error to a
  handoff between two of them. The village's lots and walls are free polygons fixed after the
  fact (leftovers absorbed, slivers moved, sharp corners chamfered, T's cut), each case making
  the next.
- **Checks that keep what exists rather than say what is right.** The accepted sets catch a
  change, not an error; the anomaly bounds pin counts per size, at one seed.
- **Too much at once,** and a change counted as done when pushed.

**The plan, in order:**
1. **An authored scene: Amazon Lumberyard's Bistro** (ORCA, CC-BY 4.0, `Bistro_v5_2`, 894 MB
   zipped): a street and a café seen through its windows, lit at night by its emissive surfaces
   alone. Its DDS textures are read as they are, block-compressed with their mips. A scene of its
   own in the physics lab (external models stay for the labs, the 2026-10-04 rule).
2. **A path-traced reference:** an offline, unbiased accumulation over the same scene, materials
   and lights, without denoising; the real-time image measured against it per view and hour.
3. **The night lighting measured against that reference,** then redesigned around one
   representation per distance band and one tested handoff, instead of tuned by eye.
4. **The private reference test of a shipped open-world city game** (ROADMAP, Phase 10) comes
   forward, after steps 1–2: one district from the owner's copy, private, never in git or
   published, compared with the game itself, its path-traced mode included.
5. **The village's layout, in parallel and on its own:** rebuilt so that bad shapes cannot occur
   (lots near-rectangular per street side, corner lots from a few templates, the rest common court;
   walls only along the streets, hedges inside; one straight line where two rows meet), its rules
   held at zero by property tests over many seeds and sizes.

**How we work from now on:**
- **One theme at a time;** an issue is done when the owner accepts it in a walk, not when its
  commit lands.
- **Design before code** for anything seen: a short spec with reference pictures and a plan or
  mock-up for the owner before the build; autonomous runs carry out agreed plans.
- **The review before the push,** as part of the gate.
- **The backlog triaged** to about 25 active issues, the rest merged or moved to "Later"; a
  session's own findings go to one triage list first.

**Amended: the engine calibrated first, then the demos** (the owner, 2026-10-30, during D-088's
SHaRC comparison: "since we changed technology we have to retest against Falcor. we will have to
do the same things with pbrt-v4. And if we try NRC, we will have to recheck again. Compare and see
what's best and work real-time"; then "Calibrate the engine first with those tools (falcor,
pbrt-v4, RTXGI, materiel test, etc.). Then update forge demos and test them."):
- **The order:** the engine is calibrated against the reference tools first. The demos (the city
  sandbox, the village, the first map) are updated after, on the calibrated engine, and tested.
- **The references**, all outside the repository:
  - Falcor's path tracer: the Bistro, the classroom, the material chart.
  - pbrt-v4: Bitterli's bedroom, classroom and kitchen (`scenes-ref/bitterli/`, downloaded
    2026-10-30); the renderer itself still to build.
  - NVIDIA's RTXGI 2.8.0 sample, run unmodified and captured from outside: its path tracer,
    SHaRC and NRC on its Bistro, LivingRoom, Bathroom, Sponza and Cornell box.
  - The material chart (`--lab chart`).
  - Forge's own path tracer (`r.gi.reference 4`) where none of them has the scene.
- **One test for every technique:** the probes, the port, SHaRC and NRC (once tried) over the
  same views and references. Each gets its error (the mean |log2| of 16 × 9 tiles, the sun and the
  sky apart) and its GPU ms on the 5070 Ti. A change of technique reruns the whole table, and the
  pick is the best that runs in real time. A repeatable tool replaces the sessions' scripts:
  `tools/gi-compare.sh` (2026-10-30; `docs/PROCESS.md`). Its views are the Bistro in clay and
  textured and at night, the classroom, the material chart, and the village's landing, room and
  street. Its references are Forge's path and Falcor. Still to add: the sun and the sky apart,
  Falcor's own scenes (the Cornell box, the grey and white room, the Arcade: the owner,
  "you can use the other falcor scenes as well"), pbrt-v4 and the RTXGI sample.
- **First findings (2026-10-30):**
  - The Bistro's lab is not metered: it draws at the shared fixed EV 14, 2.3 and 4.7 stops under
    the exposures NVIDIA's pictures fit on Forge's light (EV 11.74 at Exterior_1, 9.34 at
    Exterior_2). Its light matches Falcor's at equal exposure. Metered, Exterior_1 sits a stop
    dark and Exterior_2 on it; the tone curve is not the cause (D-089, which meters it since).
  - SHaRC reads 3 to 7 % dark against Forge's path in steady state (D-088). NVIDIA's sample's
    SHaRC is within 2 to 4 % of its own path tracer.
  - The Bistro's large tree trunk was black in every Forge render, the path tracer's too (the
    owner's eye, in the first run of the tool): its mesh, over the rays' 40,000-triangle budget,
    was traced at a coarser cut, and the rays from the drawn bark started inside it. The
    authored scenes are now traced at full detail, as Falcor's; the general case is #367 (the
    bounce's rays now trace past their own cut's back, D-088).
  - Forge's clay drew cut-out cards whole, Falcor's keeps them: clay now keeps the cut-outs.
    (Corrected 2026-10-31: Falcor's clay clears the base colour map and its alpha with it, so
    its leaf cards draw whole. `tools/falcor-render.py`'s `BISTRO_CLAY_CUTOUTS=1` keeps a
    cut-out's map, in its own colours. Between the two, the Bistro's clay moves 0.03 stops.)
- **The first baseline (2026-10-31, `tools/gi-compare.sh`, at 637abc2c):** the mean |log2| of 16
  × 9 tiles against Forge's path (`r.gi.reference=4`) and against Falcor (Lambert, 16 bounces),
  and the frame's GPU ms over 1,200 frames (three runs each). The probes are the default
  (`r.gi.cache off`); the port is set aside for its patent (D-088), measured for reference.

  | View | vs Forge's path: probes, port, SHaRC | vs Falcor: Forge's path, probes, port, SHaRC | ms: probes, port, SHaRC |
  |---|---|---|---|
  | Bistro Exterior_1, clay | 0.028, 0.046, 0.049 | 0.091, 0.099, 0.060, 0.057 | 6.72, 6.50, 7.68 |
  | Bistro Exterior_2, clay | 0.136, 0.087, 0.101 | 0.099, 0.217, 0.105, 0.104 | 5.59, 5.46, 6.37 |
  | Bistro Exterior_1 | 0.047, 0.059, 0.058 | 0.080, 0.089, 0.096, 0.096 | 6.89, 6.60, 7.87 |
  | Bistro Exterior_2 | 0.085, 0.117, 0.121 | 0.387, 0.443, 0.480, 0.484 | 5.90, 5.71, 6.70 |
  | Bistro at night | 0.508, 0.291, 0.353 | 0.983, 0.714, 0.784, 0.762 | 13.48, 13.08, 15.76 |
  | Classroom, clay | 0.368, 0.110, 0.110 | 0.090, 0.353, 0.156, 0.156 | 3.84, 3.73, 4.10 |
  | Classroom | 0.320, 0.096, 0.089 | 0.151, 0.405, 0.150, 0.149 | 4.08, 3.98, 4.34 |
  | Material chart | 0.031, 0.027, 0.028 | 0.066, 0.088, 0.084, 0.085 | 3.14, 3.14, 3.15 |
  | Cornell box (night) | 1.959, 1.871, 2.039 | 0.862, 1.100, 1.013, 1.178 | 2.96, 2.95, 3.00 |
  | Village landing | 0.507, 0.230, 0.110 | | 6.55, 6.38, 6.49 |
  | Village room | 0.147, 0.052, 0.052 | | 7.44, 7.38, 7.70 |
  | Village street | 0.116, 0.115, 0.116 | | 6.64, 6.79, 6.91 |

  - **Indoors** (the classroom, the landing, the room), SHaRC is three to five times nearer
    Forge's path than the probes, and 0.15 from Falcor in the classroom, where Forge's own path is
    0.09 to 0.15 from it.
  - **pbrt-v4 as the third reference** (the classroom's own pbrt-v4 file, and its clay made by
    turning every material into a 50 % grey diffuse; 1,024 samples a pixel, 14 minutes each on
    the CPU). In clay, pbrt is 0.111 from Falcor (×0.944); Forge's path 0.066 from pbrt
    (×1.040), between the two; SHaRC 0.081 (×0.977), the port 0.080, the probes 0.425 (×1.20).
    SHaRC's few per cent under Forge's path are partly Forge's path a little over pbrt's.
    Textured, pbrt's own materials (coated diffuse, conductors) are 0.236 from Falcor's glTF of
    them (×0.861): the converter's materials, not the light.
  - **Outdoors by day,** the probes are as near or nearer (the textured Bistro), and SHaRC costs
    up to 1 ms more there.
  - **The textured Exterior_2:** Forge's path itself is 0.39 from Falcor: the materials' gap
    (D-086), not the diffuse light's. Without any reflection on both sides
    (`FORGE_BISTRO_MATTE=diffuse`, Falcor's `BISTRO_DIFFUSE`), 0.241 (×1.008): the base colours
    hold about 0.24 of it (clay: 0.099), reflections and metals the rest. Forge is darker under
    the awnings (their undersides, the façade and the deck in their shade). Falcor's awnings
    are opaque and single-sided (no transmission): not their translucency. To find by the
    materials' albedos, view by view (Falcor's G-buffer against `r.show.input=1`). **Measured
    (2026-10-31):** the first hit's diffuse albedo, Falcor's G-buffer (`tools/falcor-render.py`'s
    `BISTRO_ALBEDO`) against Forge's `r.show.input=1`, is 0.031 apart (signed −0.008); only the
    hanging globes sit a little apart. The gap is not the albedos the camera sees: the textured
    bounce's light, and the reflections and metals.
  - **The clay rows above were not diffuse alone (corrected 2026-10-31):** Falcor's clay kept its
    dielectric layer (an index of refraction of 1.5 over the grey: 0.09 to 0.14 stops of light in
    every clay view) and Forge's its dielectric reflection (the sky's by day). Both are diffuse
    alone now (the Falcor script's clay takes an index of 1; the lab's clay `FORGE_BISTRO_MATTE
    =diffuse`'s rule), and Forge's clay keeps a room's emitters black, as Falcor's and pbrt's.
    Measured again, from Falcor (Forge's path, the probes, the port, SHaRC) and from pbrt-v4:

    | Clay view | from Falcor | Forge's path from pbrt | pbrt from Falcor |
    |---|---|---|---|
    | Bistro Exterior_1 | 0.149, 0.158, 0.117, 0.104 | | |
    | Bistro Exterior_2 | 0.192, 0.315, 0.155, 0.137 | | |
    | Classroom | 0.098, 0.434, 0.097, 0.100 | 0.048 | 0.066 |
    | Kitchen, night | 0.064, 0.128, 0.097, 0.092 | 0.044 | 0.052 |
    | Bedroom, night | 0.114, 0.849, 0.274, 0.237 | (pbrt's curtains leak) | |

    The three renderers agree in the rooms within 0.05 to 0.10. The Bistro by day does not:
    Forge's path reads 0.15 to 0.19 stops over Falcor's there, which Falcor's layer had hidden
    (#373: the normal maps' darkening, two-thirds of it taken since, D-086). pbrt-v4 lets the bedroom's panels through its curtains: their folds' smooth normals
    put a shadow ray behind the sheet on the lit side of its shading normal.
  - **The night and the Cornell box, as first measured, were not trustworthy;** two causes found
    the same night (2026-10-31):
    - **The box was too bright for the night's units.** At Falcor's unit (12,800 cd/m²) its
      lamp-lit floor stood at 160,000 of the shading's units at night (cd/m² × π over the moon's
      0.17 lux), past the half floats the probes pack their rays in (60,000): they read it at a
      third, and the light quad at nothing. At 30 cd/m² a unit (a room lit at night) every value
      fits. Forge's storage at night has no headroom for such light: an issue.
    - **An emitter that is a lamp counted twice.** The probes leave a lamp row's glow out
      (D-065); the bounce, the port and SHaRC's update added it on top of the lamp (the box's
      floor at twice Falcor's): they leave it out now (`ray_hit_lit`), and the resolve's
      reference (`path_from`) too. The tool captures Forge's path with the lamps off all the
      same, its rays meeting the glow itself, so the lamps' faults (#369) stay out of it.
    - **#355 seen again, and its oracle found:** the reference's change moved every asteroid
      capture, while those scenes never run it. The AMD iGPU (`FORGE_GPU=amd`) draws the
      asteroids alike with and without it; NVIDIA draws them as AMD only with it. Without it the
      rocks lose their shadows on each other: the images accepted since 2026-10-29 were the
      broken state, accepted again with the shadows back (the slangc output valid both ways,
      `spirv-val --scalar-block-layout`).
  - **Measured after both (frame 280, three techniques one run each):** the Cornell box, Forge's
    path 0.025 from Falcor (×1.000); the probes 0.322, the port 0.255, SHaRC 0.261 (×0.93 to
    0.96). The Bistro at night, Forge's path 0.317 (signed −0.010); the three techniques 0.549,
    alike: the lamps' direct light rules there.
  - **Left, the lamps' direct light:** on the box's back wall and its tall block's front (facing
    the camera) a lamp gives half of Falcor's direct light (11.4 against 21.8 cd/m²), the floor
    and the side walls within 20 %. Not the profile alone (a flat piece's lamp falls off by a
    smoothstep, `Emission::flush`, which near the horizon gives half a diffuser's cosine, but the
    back wall sees the lamp at about 49°). Found the same night (#369): the lamp's shadow rays
    aim at a disc facing the surface, which rises into the ceiling (a 5 cm radius took the back
    wall from 11.4 to 16.8), and the profile near the horizon does the rest. Both are in the
    resolve (#355): to plan.
  - **Two rooms by night (2026-10-31):** Bitterli's bedroom and country kitchen, their pbrt-v4
    files through `tools/pbrt-to-gltf.py`, their area lights as lamps (D-087), at 30 cd/m² a
    unit (`gi-compare.sh`'s `bedroom`, `kitchen` and their `-clay`). Forge's path is 0.098 and
    0.110 from Falcor in clay, 0.392 and 0.206 textured (the converter's materials again). The
    three techniques are far over it: about ×6 in the bedroom, a stop in the kitchen. Both
    rooms' main lights are large panels behind fabric (the bedroom's curtains, the kitchen's
    blind; opaque in Forge and Falcor, the converter leaving pbrt's `diffusetransmission` out),
    and a flat piece's lamp was a disc of its area centred half its radius in front of it: 39 cm
    and 53 cm into the rooms, past the fabric, unshadowed (#370, the root of #369's too). The
    GPU's frame, one run: 4.8 to 5.1 ms with the probes, 4.7 to 5.0 with the port, 5.1 to 5.4
    with SHaRC.
  - **With a planar piece's lamp on its face (D-087's amendment, #370):** the kitchen's clay
    0.10 to 0.15 from Falcor (the port 0.102, SHaRC 0.114, the probes 0.146), textured 0.25 to
    0.33. The bedroom −1.2 to −1.6 stops: its lamp's disc and its light from the middle of a
    large panel lose the light that escapes round the curtains (Forge's path with the lamps on
    is a stop under itself with them off). The rooms measure the techniques where the lamp holds
    (the kitchen); the bedroom waits for the rectangle (#369).
  - **With the rectangle (#369):** the lamp holds in all three rooms (Forge's path lamps on
    against off: 0.021, 0.073, 0.097), so they measure the techniques. **SHaRC is the nearest
    indoors at night as by day:** the bedroom's clay 0.237 from Falcor (the probes 0.849; the
    port unstable there, #371), the kitchen's 0.092 (the port 0.097, the probes 0.128), the
    box 0.096 (0.092, 0.207); the clay against the corrected references (below). The bedroom's light comes out from behind its curtains after many
    bounces in a 27 cm gap, which the probes a metre apart cannot hold: D-090's case for SHaRC
    indoors, at night too.

**The pbrt-v4 pass (2026-11-01; the owner: "the pbrt-v4 simple pass with what we downloaded [...]
(profiling stats are welcome)"):** pbrt-v4 as the third reference column of `tools/gi-compare.sh`
(`PBRT_OUT`, the views' `pbrt_ref`), its renders made by `tools/pbrt-render.sh` through
`tools/pbrt-variant.py`, which rewrites a scene as the converter's glTF gives it to Forge and Falcor:
`--flat` (the meshes' shading normals dropped: the converter keeps each face's own) and `--opaque`
(`diffusetransmission` made diffuse: the converter leaves the transmission out, so the bedroom's
curtains and the kitchen's blind are opaque in both). Bitterli's bedroom in pbrt had let light
through its curtains both ways (smooth normals on the folds in clay, 45 % transmission textured).
1,024 samples a pixel at 1280 × 720 on 12 threads: 9–21 minutes a room on the CPU (classroom clay
538 s, textured 1,064 s; bedroom clay 750 s, textured 1,253 s; kitchen clay 680 s, textured 900 s),
against Forge's path's 300 frames. The mean |log2| of 16 × 9 tiles (frame ratio), Forge's captures
of 407520d8:

| View | pbrt-v4 from Falcor | Forge's path from pbrt-v4 | probes, SHaRC, NRC from pbrt-v4 | GPU ms: probes, SHaRC, NRC |
|---|---|---|---|---|
| Classroom, clay | 0.069 (×0.99) | 0.056 (×1.03) | 0.403, 0.093, 0.101 | 3.89, 4.23, 5.85 |
| Cornell box, night | 0.055 (×1.01) | 0.071 (×1.02) | 0.181, 0.080, 0.072 | 2.99, 3.10, 4.34 |
| Bedroom, clay, night | 0.048 (×1.02) | 0.079 (×0.98) | 0.808, 0.202, 0.235 | 5.04, 5.57, 6.66 |
| Kitchen, clay, night | 0.054 (×1.00) | 0.049 (×1.01) | 0.107, 0.069, 0.075 | 5.19, 5.78, 6.90 |
| Classroom | 0.234 (×1.17) | 0.326 (×1.24) | 0.600, 0.243, 0.293 | 4.14, 4.43, 5.97 |
| Bedroom, night | 0.553 (×1.14) | 0.409 (×1.17) | 0.676, 0.257, 0.244 | 5.45, 5.98, 7.10 |
| Kitchen, night | 0.340 (×1.04) | 0.280 (×1.04) | 0.216, 0.246, 0.236 | 5.59, 6.29, 7.39 |

(The GPU ms: the frame at the views' size under DLAA, two runs of 1,200 frames, D-090's.)
- **In clay the three renderers agree** within 0.05–0.07 in all four rooms, the bedroom included
  now its geometry is the same in all three (pbrt's old render: 0.21 from Falcor), and Forge's path
  lies within 0.05–0.08 of pbrt-v4: the calibration's diffuse light holds against a third, CPU,
  renderer. The techniques rank as against Falcor and Forge's path: SHaRC and NRC within 0.07–0.24,
  the probes 0.11–0.81.
- **Textured, pbrt-v4 reads 4–19 % darker** than Falcor and Forge: its coated diffuse (a dielectric
  coat over the base, reflecting and absorbing) and its conductors against the converter's
  metal-roughness approximation of them. A gap of the converter's materials, not of the light;
  closing it is `tools/pbrt-to-gltf.py`'s, when the materials are next calibrated.

**Amended: after the calibration, the private district as a game in production** (the owner,
2026-11-01: "We will work on the pbrt-v4 simple pass with what we downloaded for the moment unless
there are big issues to fix (profiling stats are welcome). [...] Once pbrt-v4 done and before any
other scene tests, we will start to work on the [...] private demo. The goal is will be to start
working as if we were working on a proper game engine."):
- **The order:** the pbrt-v4 pass over the scenes already downloaded (Bitterli's rooms and the
  Cornell box as pbrt-v4's third reference column in `tools/gi-compare.sh`), then step 4, the
  private district of a shipped open-world city game, before any other scene test.
- **The district** is worked as a game in production: its own data through Forge's pipeline, held
  to D-094's frame, and compared with the shipped game on the same machine. The owner captures
  the game's pictures and its benchmark's frame rate from a shot list; nothing of it enters the
  repository (ROADMAP, Phase 10).
- **A forest reference for later:** a free scene of trees, grass and undergrowth that Falcor,
  pbrt-v4 or Cycles path-traces and Forge loads, found now and downloaded on the owner's word
  (the foliage work of D-075 and D-091 measured against it).

**The district against its source's frames (2026-11-04/05).** The source's benchmark frames are
matched to the path by their edges (frame k at (k − 11) × 2 + 1 s; 25 of 37 match). The district
takes its source's exposure limits (D-089's note), its lamps against Forge's physical sun, its
glows' ray-traced EV, and its sun's colour, each fitted or read on those frames (the numbers stay
with the private data). Two engine changes came of it:
- **A scene's sun had the default sun's colour.** City-blocks coloured the sun through the air at
  the default 63.4° and kept that colour when a scene gave its own direction: the district's
  afternoon sun at 34° and the village's at 33.5° were lit as at noon. The colour is now the
  air's at the scene's sun. The map check sets the hour (`--time-of-day`) and is unchanged (ꟻLIP 0,
  13 shots); the village run without it gets a slightly warmer sun, and the physics lab's yard and
  materials scenes (a sun 26.6° up) a redder one, about 0.2 EV dimmer at a fixed exposure: their
  captures (`lab-yard*`, `lab-materials*`) move. `--sun-elevation` already left a scene's sun's
  direction alone; now its colour too.
- **A scene may give its sun's colour** (`Look::sun_color`, its hue at the air's level): its
  authors' light where the air's model differs (a white sun wins over it; `--day`,
  `--time-of-day` and an environment map colour the sun themselves). The district's afternoon sun is golden: its
  sunlit streets' red over green 1.07 → 1.36 against the source's 1.49 (blue over green 0.88 →
  0.69, the source 0.65).
- **A scene may give its sun's level** (`Look::sun_illuminance`, lux, the sky's light with it): an
  authored world's lamps are set against its own sun. The district's interiors, at its source's
  exposure floor, wanted 2.17 EV more lamp light than its lumens give under Forge's 128,000-lux
  sun (six matched frames, ±0.24). Its lamps ×4.5 matched them but every lamp's reach (where it
  falls to `LEAST_LUX`) grew 2.1 times, and the bar's fog pass with it (1.2 → 3.8 ms, the frame
  10.7 → 14.0 ms there): a sun ÷ 4.5 gives the same picture, the lamps as their data gives them.
- **A lamp may end at its authored range** (`Lamp::range`, `Lamp::reach`): its light fades to
  nothing there (the shaders' window), never past the reach to `LEAST_LUX`. The source's lights end
  at their attenuation radius, 2 to 8 m for most of the bar's, where that reach ran 20 m and more:
  the bar's frame at 1600 × 900 went 13.4 → 9.0 ms at night (`fog/light` 3.30 → 0.69, ReSTIR, the
  probes' rays and the shading with it). Its lamps then light the bar in pools as the source's do,
  and the balance against the sun was fitted again (its lamps about 5 EV over a physical balance
  with the sun, near the exposure's own offset). No other scene gives one.
- **Whether the reference's 32 rays can be trusted (2026-11-06, #397):** yes. One still in the
  private district read flat at 32 rays and right at every other count. The cause was one wrong
  binary in NVIDIA's on-disk pipeline cache, under that viewer's exe name, not the ray count: the
  same SPIR-V compiled cold reads right.
  - **Void:** only that viewer's 32-ray `r.gi.reference` stills of 2026-11-06, modes 1 to 6.
  - **The references stand:** the baseline and the pbrt-v4 and Cornell agreements were taken under
    older drivers and caches, and the 140 stored path logs keep their views' costs (the fault
    costs 0.2 ×). They were not re-run. At HEAD, the Bistro's two views and the Cornell box at 32
    rays match 33 rays in modes 2 and 4, to 0.03 of a luma code at most.
  - **The check:** `docs/PROCESS.md`'s debugging aids.

## D-084 — An environment map as the sky, its sun taken out of it ✅ (the owner's go, 2026-10-29)

**Why:** D-083's first comparison, the Bistro's Exterior_1 beside NVIDIA's render, could not
match while Forge lit it with its own atmosphere. The authors lit the scene with an HDR
photograph (HDRI Haven's San Giuseppe bridge, at Falcor's intensity 10). Their render is
path-traced under that photograph alone; the FBX's directional light is for rasterisers, the
archive's readme says. Engines take such skies as a matter of course, and every reference made
under one needs it. The owner chose this over keeping Forge's sky ("our engine has to learn to
use the bistro technology indeed").

**What** (`forge_render::environment`, `sky.slang`; a scene asks with `Look::environment`):
- **The map:** a Radiance HDR file, read in Falcor's equirectangular layout (u from atan2(x, −z),
  v from the zenith down). Stored as RGB9E5 (four bytes a texel) with box-filtered mips: about
  45 MB for 4096 × 2048.
- **Its sun:**
  - At load, the texels round the brightest one that hold over 40 times the map's mean are
    summed by solid angle. That sum is the scene's sun: its direction, its colour, and the
    reference illuminance. Their spread gives its size for the soft shadows, in place of
    D-049's 1° (`--sun-size` still sets one).
  - Those texels are then held to the bound, so the sky keeps no second sun.
  - The sky is scaled against the sun, so the photograph's balance carries over. San Giuseppe:
    its sun, 28.6° high, gives about as much light on the horizontal as the rest of the sky.
- **The sky's passes:**
  - The sky-view table is filled from the map at the mip its texels match. The sky's irradiance
    (the nine harmonics), the probes' misses and the resolve's reflections follow from that
    table, as before.
  - The compose pass draws the map behind the scene at full resolution and hazes nothing.
  - No clouds, no night.
- **The resolve shader is untouched:** it stands near the size where its ray queries fail
  (#355; a branch added there on 2026-10-29 moved every asteroid capture).

**Measured on Exterior_1** (the camera solved from six points, 4.5 px):
- **The sun's direction:** of eight candidate readings of the map's layout, rendered from that
  camera, Falcor's correlates 0.82 with NVIDIA's frame. The FBX sun scores 0.59, and the other
  readings 0.50 or less.
- **Brightness:** at EV 11.4 (2.6 stops over the lab's 14), sunlit and shaded cobbles hold the
  same ratio as in NVIDIA's.
- **The emissive maps** take the photograph's scale: Falcor lights them at 1 and the photograph
  at 10, so a full emissive texel is 0.1 of the photograph's units, 2,870 cd/m² here.
- **Darker, crisper shadows and warmer colours in NVIDIA's** were mostly its tone curve
  (Narkowicz's ACES fit, where Forge defaults to AgX punchy) and Forge's widened sun (D-049).
  The sun now keeps the photograph's size too, 0.29° in radius (√2 times its texels' RMS
  angle, weighed by their light, never under the real sun's); under ACES the two come close
  (`docs/demos/physics-lab.md`, `bistro`).
- **The owner's choice (2026-10-29):** the Bistro takes ACES as Falcor's ToneMapper applies it
  (`aces-narkowicz`, Narkowicz's fit after a 0.6 pre-exposure; a scene's look may now pick its
  tone curve, `Look::tonemap`; `--tonemap` and `r.tonemap` override it) and keeps the
  photograph's sun, to see how close Forge comes to the authors' renders.
- **Its level through the atmosphere (2026-10-31, the owner: "is the sun intensity what we
  actually receive on earth?"):** it was not. The photograph's sun was given the sun's light above
  the atmosphere, 128,000 lux, and the sky with it: a street 1.4 to 1.8 times a clear day's. Now
  the Bistro's (`Environment::through_atmosphere`) is dimmed as Forge's own sky dims its sun, by
  its atmosphere's transmittance at the sun's height (its luminance: the photograph keeps its
  colour and its sky against its sun): 0.749 at the photograph's 28.6°, 95,900 lux. The metered
  pictures are as before; at a fixed exposure 0.42 stops darker (`tools/gi-compare.sh`'s Bistro
  views by day at EV 10.80, Falcor's unit 2,150 cd/m² in place of 2,871; Exterior_1 against
  Falcor as before, 0.109 in clay). The glow keeps its 2,870 cd/m² (the night's, calibrated
  against Falcor's); the chart and the rooms stay at Falcor's units, as they are compared. Where
  the photograph was taken (HDRI Haven's San Giuseppe bridge) and its hour are its own: the
  street is modelled on Paris, where the noon sun stands at 28.6° about 20 February and 21
  October.

**Not yet:**
- Sharp mirrors see the sky through the table (about a degree a texel).
- A map's rotation, if a scene needs one.
- Light from the map's ground below the horizon reaches the scene only through the harmonics.

## D-085 — A metal's highlight weighs its reflectance and its lobe ✅ (the owner's ask, 2026-10-29: "now fix the white lamp post")

**Why:** a metal's sun and lamp highlight weighed 0.35 more than a dielectric's (0.4 in all),
whatever the metal. The Bistro's lamp post, a dark rough metal (reflectance 8.5 %, roughness
0.74), shone white beside Falcor's path-traced render of the same view (D-083 step 2):
- **Measured:** the post's light over the sunlit cobbles' was 0.58 in Forge and 0.14 in Falcor.
  A white diffuse post in both put 1.6 of that at the light round it, the rest in the metal.
- **Ablated:** the 0.35 off, the post fell from 0.0489 to 0.0071 (EV 15, Khronos Neutral)
  against Falcor's 0.0145. The sky's reflection and Fresnel at grazing angles moved nothing.
- **Not the lobe's width:** Falcor's post at roughness 1 is darker still (0.094), not brighter.

**What** (`forge_core::material::metal_highlight`, `uv_mapped_light` in `meshlet.slang`):
- A metal's highlight weight is its reflectance at normal incidence (its base colour's brightest
  channel, as its reflection already was) times the normalisation of a Blinn-Phong lobe of its
  power, (n + 8) / 8 (Akenine-Möller et al., *Real-Time Rendering*), up to 8 (power 56).
- **The cap** keeps a polished metal's sun glint a highlight rather than a sparkle the
  anti-aliasing cannot hold; the owner may raise it.
- Dielectrics keep their 0.05. Only metals change: the glTF and Bistro rows (UV-mapped) and the
  flat glTF rows. The procedural kits set their own weights.
- **Measured after:** the post 0.0147 against Falcor's 0.0145. The asteroids (#355), the lab's
  creatures and materials: 0 px.

**Not yet:** the light round the post looked about 1.4 times Falcor's, measured before the
Neutral curve's black offset was undone: to measure again. The highlight is still Blinn-Phong,
untinted, and without the light's cosine.

## D-086 — Forge's surface model, measured against Falcor: a microfacet BRDF for every light ✅ (proposed and decided 2026-10-30)

**Decided (the owner, 2026-10-30: "about the decisions I will follow recommendations"):**
- **Diffuse: Lambert, kept.** Every comparison runs Falcor with its diffuse switched to Lambert
  (the local define), so the gaps measured are Forge's specular and light. Next to NVIDIA's own
  pictures rough surfaces stay up to ~1.5 times brighter: a choice, not an error (Frostbite's
  renormalised Disney is a convention; a white rough tile head-on reflects ~0.66 of a real white
  diffuser). With the specular fixes in, the chart and the Bistro both ways for the owner to
  revisit.
- **The BC7 encoder crate:** approved, free and royalty-free, credited, its patents checked first
  (and Kulla-Conty's or Fdez-Agüera's compensation's).
- **The rest as proposed**, in its order, with the lamps' size added (below).
- **The rows' highlight weights** (RenderLayer's `specular`, tuned under Blinn-Phong from 0.02 to
  0.8): a row's GGX highlight takes `min(specular / 0.05, 1)` of itself. Weights under the
  default (a bench, damp prints, rubber) keep a dimmer highlight; those over it made up for the
  unnormalised lobe, which GGX's normalisation gives.
- **The lamps' size** (the owner's side session, 2026-10-30, on the scooter "flat especially
  during the night"): a lamp's sphere widens its GGX lobe as the sun's disc does (its angular
  radius from the surface), so a smooth surface's highlight of a bulb is the bulb's size, not a
  sub-pixel point; and the lamps' highlights take the pixel's own roughness, normal and
  reflectance (today the row's factors before its maps: roughness 1 on every mapped row).

**Step A, the sun's highlight (2026-10-30):** GGX with the height-correlated Smith visibility and
Schlick's Fresnel from the row's reflectance, its lobe α² = 2 / (power + 2) widened by the normals'
spread under the pixel and by the sun's disc ((θ/2)²). The chart, sun at 30° from the side, against
Falcor in Lambert at 16 bounces: the grey spheres 0.86–1.02 → 0.91–0.93, the red 0.76–1.16 →
0.88–0.93, silver 0.21–5.56 → 0.74–2.13, gold 0.28–6.72 → 0.82–2.77 (the rough metals' rest is
the environment's, step C), the dark tile 1.27 → 1.00, the ground 1.26 → 1.00, the white tile
0.96. The Bistro's Exterior_1: the sunlit street 0.92–1.07 → 1.04–1.17 (its cobbles' highlight;
their BC5 normals still flattened, step E); a still camera 5,331 pixels against 5,245; the frame's
cost unchanged (4.60 ms).

**Step B, tinted metals:** F0 per channel, a metal's its base colour, through the sun's highlight
and the sky's and probes' reflection (a traced mirror ray's share keeps one weight, its
luminance: the request image holds one). The chart's gold at roughness 0.2: (2.26, 1.76, 0.86)
against Falcor's (2.53, 1.92, 0.84).

**Step C, the environment by the split sum:** GGX's directional albedo from Lazarov's analytic
DFG fit (Karis 2014), and Fdez-Agüera's multiple-scattering compensation (JCGT 2019; no patent
found in a search, used by Blender's EEVEE and VTK), switchable for the comparisons
(`r.specular.single_scatter`). The chart: with single scattering as Falcor's, silver 0.73–1.15 and
gold 0.65–1.17 across roughness (0.21–5.56 and 0.28–6.72 before D-086); with the compensation
(the default) rough metals keep their light, silver 1.30–2.04 and gold 1.21–1.90 at roughness 0.8
and 1, Falcor's lost energy; dielectrics 0.88–0.93, the tiles 0.95–1.00. Open: smooth metals at
0.66–0.74. The Bistro's Exterior_1 against Falcor in Lambert: the sunlit street 1.01–1.06 (0.92–1.07
before D-086); a still camera 5,032 pixels (5,245 before D-086); the frame 4.61 ms, unchanged.

**Found (2026-10-31, the owner: the two smooth gold balls had lost their gold):** a
smooth row's mirror ray took its Fresnel's luminance alone (the request's one weight), so its
traced share reflected grey: gold's red at 0.85 of Falcor's, its blue at 1.35, green over red
0.85 and blue over red 0.57 where Falcor's are 0.77 and 0.36. The request image now carries the
colour in a lower half (`g_reflection_tint`, the Fresnel over its luminance), and the mirror pass
and the resolve's sky under it take it: green over red 0.76, blue over red 0.35, red 0.93 of
Falcor's (the two balls' lit pixels, without anti-aliasing). **And a metal in a mirror (the owner,
the same day: the neighbours in the smooth balls flat discs of their colour):** a mirror ray's hit
was lit as paint, its base colour under the sun and the probes, so a metal ball seen in another
read as a flat yellow or white disc. A metal hit now reflects (`hit_metal_light`): GGX's
directional albedo of its base colour times what its mirror direction meets (above the horizon
the sky's table dimmed as the probes see it dimmed, below it the probes' light along it: the
floor; the map's lower half is dark), and the sun's glint, its shadow ray traced. The probes'
lookup turns from the hit's normal to its mirror direction for a metal, one lookup either way
(#348). What the metal's own reflection meets of the scene, and the lamps' glints on it, are
left out. The smooth gold keeps 0.77 and 0.36; the chart's `shading/reflections` 0.048 →
0.060 ms (one run each).

**Normal maps darken the diffuse light (#373, 2026-10-31):** Forge's path read the clay Bistro 9 to
10 % over Falcor's by day; under its sun alone, Falcor without normal maps against Forge, ×1.004
after one bounce and ×1.013 whole, where with them ×1.093 and ×1.111. A Lambert surface's light
from one direction is linear in its normal, so over bumps it is their mean normal's: the
cobbles' map (decoded) holds a mean normal 0.900 long, and its bumps catch 0.929 of a flat
face's sun at the photograph's 28.6°; Falcor's sunlit ground loses 8.4 % to its maps. Forge lost
it twice over:
- **at a ray's hit,** which took its face: now the map's mean normal (its 1 × 1 mip's, the mips
  rebuilt from the unit normals, #331), turned into the world by the triangle's UVs (the blended
  pavement's leans 0.12 off its face), its cosine the sun's (`RayHit::lit_n`) and its length
  the lamps' share;
- **at a pixel far off,** whose filtered normal was made unit and lost its bumps: the pixel's
  diffuse albedo now takes the filtered normal's length past the bytes' rounding (1 where the
  texels show), less as `normal_strength` flattens the bumps.

The review the same day: the hit's share went into its albedo (the probes' light and the bounces
after take it, as the pixel's do), its tilt only on rows read by UVs; a weak map (a strength
under 1) had darkened even where the texels show; `r.show.input=1` shows the albedo before it.
And the mirror rays' metals: a mapped row's metalness factor is 1, its map the rest, and the
hits read the factor alone, so every mapped row of the textured Bistro reflected as a metal in
its windows: the hit takes the map's mean now. A metal's probes are looked up along its normal
(along its mirror direction the lookup leaned into the wall behind).

Forge's maps took 3 % off the Bistro's light, 5.5 % now, Falcor's 8.1 %. Under the sun alone
×1.093 → ×1.052 after one bounce, ×1.111 → ×1.067 whole (both without maps: ×1.023, ×1.030).
Without anti-aliasing, from Falcor (Forge's path, the probes), with the review's fixes below
(the hits' share in their albedo): the clay Exterior_1 0.161 → 0.085 (×1.097 → ×1.047), 0.158 →
0.081; Exterior_2 0.206 → 0.108 (×1.101 → ×1.050), 0.320 → 0.223; textured Exterior_1 0.082 →
0.079 (×1.011), Exterior_2 0.378 → 0.399 (×0.962, its materials' gap); the chart as it was. The sentinels 0 px (none draws a UV-mapped normal
map); the village's map check: its tiled roofs a few levels darker along their rows from afar
(at most 0.5 % of a view's pixels), accepted.
Left: Falcor's rays leave each bump round its own normal and lose those under the surface,
which Forge's flat bounces do not. `FORGE_BISTRO_NO_NORMALS=1` and the Falcor script's
`BISTRO_NO_NORMALS` take the maps away, and `r.gi.reference` 5 to 8 stops Forge's path after 1
to 4 bounces (Falcor's `BISTRO_BOUNCES`).

**Step D, the lamps:** their highlights by GGX, each lobe widened by its lamp's sphere as seen from
the surface (half its angular radius on α, as the sun's disc), Fresnel from the pixel's tinted F0,
and the standard rows' inputs (normal, lobe after Toksvig, reflectance) worked out before their
lamps (`standard_inputs`, `row_inputs` for the other classes, one `lamps_at` still): the lamps had
taken the row's factors, roughness 1 on every mapped row, and Blinn-Phong's fixed weight. The
Bistro's night: the scooter's paint glossy with the lanterns' highlights and its chrome bright,
where both were matte (the owner's "flat especially during the night"); with all 132 lamps 14.3 ms
(13.6), the resolve +0.4 ms; a still camera's frames 8,162 pixels apart (6,217): the thin geometry
under DLAA's jitter with brighter highlights (chairs, wires, trims). By day 5,032, unchanged.

**Step E, the normal maps:** a BC5 normal map's levels rebuilt from its level 0's unit normals,
unnormalised (each the mean of the four above), and encoded as BC7 (`intel_tex_2`, Intel's ISPC
Texture Compressor, MIT; BPTC has no known IP claims in Khronos's registry): a coarse level's
shortening reaches Toksvig, so the highlight widens where the bumps average away. The Bistro's
132 maps in 13.3 s once, then 75 ms from `bc7-normals/` beside the converted scene (598 MB);
`FORGE_BISTRO_BC5=1` keeps BC5. Exterior_1 against Falcor in Lambert: the fronts and the street
0.85–1.28 (to 1.9 before); the shade under the awnings 1.1–1.7 (D-088's); a still camera 5,004
pixels (5,032). A mip bias of −1 (NVIDIA's DLSS guidance for DLAA, `r.textures.lod_bias`, 0 by
default) sharpens the cobbles a little for 10 % more shimmer (5,502): not taken. Falcor's
cobbles keep more contrast (its 1,024 jittered samples of level 0). **BC1 normal maps alike**
(2026-11-04, `bc1_normals_to_bc7`): a production's tools write BC1 normal maps (RGB, the normal
in every level made unit), four in five of the private district's layered materials' maps; read
as they were, its metals and tiles stayed mirror-smooth at any distance. Rebuilt the same way, a
tiled wall's grout widens its lobe from a few metres; up close, where the texels are larger than
the pixels, nothing changes, as it should.

**The review's fixes (2026-10-30):** the GGX terms in `shaders/brdf.slang`, shared by the
resolve and the impostors (still Blinn-Phong and Lagarde's Fresnel, they would have stepped in
colour where the cards hand over to them; their sky's reflection at the leaves' mean n·v from
their baked Schlick term); a tinted metal's traced share at one weight for the open sky it
replaces too (a sky-coloured rest, negative in blue on gold); the lamps chosen and shadowed by
the interpolated normal, lit by the mapped one (a mapped normal leaning towards a lamp round its
face's corner lit it unshadowed); a pane's room's daylight no longer drops the pane's mirror
ray; BC7's error not read as spread (0.02); the DFG fit's B gated under an F0 of 0.02 (Unreal's);
a metal's highlight whole; the debug views without the reflection; the BC7 cache written whole,
normal maps alone.

**The look's change (to accept or tune):** the village from the air, against this branch's
shaders before D-086: 42 % of pixels change (mean 4.3 of 255), its grass and trees darker: the
rough surfaces' fixed 5 % of the sun is gone (the chart's dark tile 1.27 → 1.00). The tuned
scenes' colours to look at with the owner before a push.

**Step F, the debug views:** `r.show.input` shows a standard row's shading input in place of its
light, before the tone curve: 1 the diffuse albedo, 2 the perceptual roughness after Toksvig, 3
the metalness, 4 the shading normal, 5 the lobe's widening (Toksvig's variance red, the geometry's
under the pixel green, × 4), 6 Fresnel's F0. In the Bistro the cobbles read rough (about 1) a few
metres off, their bumps' spread widening the lobe as step E meant. Still to come: the GPU test per
normal-map format (the CPU test checks the levels' shortening).


**Why:** the owner asked how Forge knows it is accurate ("sometimes I find sun light reflection
to be too bright, losing the details of what the light is reflecting off"). Until Falcor, by
eye: the resolve's lighting is still Phase 0's model. The material test chart (`--lab chart`,
D-083 step 2) now measures it, Forge's `--capture-linear` against Falcor's EXR of the same file
under six skies (`docs/demos/physics-lab.md`, `chart`). Forge over Falcor, sun at 30° from the
side:

| | smooth (0.05) | rough (1) |
|---|---|---|
| grey dielectric | 0.90 | 1.47 |
| red dielectric | 0.79 | 1.69 |
| silver metal | 0.21 | 5.7 |
| gold metal | 0.28 (untinted) | 6.8 (untinted) |

The white tile 1.49, the 18 % tile and the ground 1.81. With Falcor's diffuse switched to
Lambert (a local define): the white tile 1.03, the grey spheres 0.86 to 1.02, the dark tiles
1.27. Falcor at 16 bounces instead of 3: no change on the chart.

**What the numbers say:**
1. **Diffuse:** Falcor's is Frostbite's renormalised Disney model (darker with roughness, to
   1 / 1.51), Forge's Lambert. A choice, not an error: either can be right for the look.
2. **A dielectric's sun highlight** weighs a fixed 0.05 with no Fresnel and no n·l: at roughness
   1 its lobe is flat, so it adds 5 % of the sun everywhere (dark rough surfaces +27 %), and at
   a low sun the glare towards the light is too weak.
3. **Metals:** untinted (gold renders silver); a smooth metal's sun glint is missing (D-085's
   cap); a rough metal behaves as a white diffuse. Falcor's single-scatter GGX loses most of a
   rough metal's light, which a real one does not: neither is right at roughness 1.
4. **Normal maps in BC5** (the Bistro's): read as (x, y, 1), Toksvig never widens their
   highlights and their slopes are flattened (45° reads 35°): highlights too sharp and bright
   where the mips average the bumps away (the owner's "sometimes").

**The proposal:**
- **One specular model for the sun, the lamps and the environment:** GGX (Trowbridge-Reitz)
  with height-correlated Smith visibility and Schlick's Fresnel, F0 from the base colour for
  metals (tinted) and 0.04 for dielectrics; the sun as a disc, its angular size bounding the
  glint (no cap). Rough reflections of the sky and the probes by the split-sum's DFG table
  (Karis 2013); mirror rays below roughness 0.42 as now.
- **Multiple-scattering compensation** for GGX (Kulla and Conty 2017, or Fdez-Agüera 2019): a
  small table, rough metals keeping their light. The chart then compares against Falcor with
  the compensation off (Falcor has none) and documents the difference.
- **Diffuse:** keep Lambert or take Frostbite's (the owner's choice, by the chart and the
  Bistro side by side). Frostbite's matches the reference; Lambert keeps Forge's look.
- **BC5 normals:** transcoded at load to BC7 with mips averaged from the unit normals (their
  length kept, so Toksvig works as for RGB maps), cached on disk; no resolve change. A crate
  for BC7 encoding (MIT) to choose.
- **Debug views** of each shading input (base colour, roughness, metalness, normal, Toksvig
  variance, F0), and a GPU test per normal-map format (a bumpy map's coarse mip has variance).

**Cost:** the resolve's shading grows (GGX and the DFG table instead of Blinn-Phong): to be
measured on the Bistro and the village; the #355 guard (ast240, the lab's creatures) after each
change. One theme, several commits, each checked on the chart.

**Patents:** GGX, Smith, Schlick, the split sum and Frostbite's diffuse are published without
known patents; Kulla-Conty's and Fdez-Agüera's compensation and a BC7 encoder to be checked
before building (the owner's rule).

**Also measured, not part of this:** the light in shade, under awnings and along fronts, is 1.2
to 1.9 times Falcor's in the Bistro once the diffuse model is matched (Lambert, 16 bounces); the
bounces and the materials ruled out (clay renders show it). It is the probes' resolution at the
fronts: the Bistro's finest cascade is 4 m; 1 m over the whole view (64 × 16 × 64 probes) brings
the café's front to 0.9 to 1.4, at 8.6 ms and 464 MB; 1 m within 24 m, or 2 m, changes nothing;
a 5 m occlusion radius (GTAO) takes it to 1.0 to 1.3, too dark in places. The real-time answer
(occlusion traced at medium range, or probes placed by the geometry) is a design of its own.

## D-087 — The night's light from its emissive surfaces: emitters as lamps, probes without them ✅ (proposed and decided 2026-10-30, the owner following the recommendations; D-083 step 3)

**Why:** the owner: "emitting lights are less bright in our engine" and, of a night run, "it's
shimmering everywhere, light is not stable". NVIDIA's Night picture, its camera solved from six
points (`--view=-16.735,5.218,2.083,-80.44,-9.63 --fov 55.99`, 2.1 px), is Falcor's path tracer
lit by the emissive surfaces alone at +9 stops (mean luma 53.4 against NVIDIA's 50.7; EV 2.2 in
Forge's units, a street at night). Forge at the same view and exposure (`--time-of-day 1.5`):
- **The emitters themselves:** 0.70 of Falcor's light (0.83 by day).
- **The light they throw:** 0.04 to 0.3 of Falcor's on the café's front, the terrace and the
  street. Forge carries emitted light only through its probes, whose 128 rays a frame seldom
  meet a lantern's glass or a bulb.
- **The flicker:** a still camera changes 261,857 pixels (18 %) between frames 10 apart; 1,640
  with the probes off. The probes' rays meet the small bright emitters by chance, and the night's
  exposure shows it everywhere. By day: 5,361 (0.37 %), on thin geometry (baskets, chair legs,
  wires) under DLAA, unchanged without the probes' spacing, the sun's size, mirror rays or the
  normal maps.

**The proposal:**
- **Emitters as lamps (D-065's):** at load, each emissive section of a mesh split into its
  connected pieces (a lantern's glass, a bulb, a sign); each a lamp at its middle, as wide as the
  piece, its luminous intensity from its luminance and area (a closed bulb L × A / 4 every way, a
  flat sign L × A along its normal in a hemisphere cone), its colour its map's mean. Lamps take
  an RGB tint over their colour temperature (CPU side only: the GPU record already holds RGB
  candelas).
- **The probes' rays skip emission** where the emitter is a lamp: its light reaches the probes
  as the lamps' light does, sampled, not by chance. No flicker.
- **Choice and shadows as D-065's:** the 64 nearest by lists, the street lamps (outside) by
  ReSTIR DI, the owner's choice of 2026-10-18 (its patents as settled then). The Bistro's string
  lights, a few hundred bulbs, test the many-lights path.
- **The archive's night without Forge's moon:** a look switch for comparisons (its readme: the
  night is its emissive surfaces alone); Forge's own nights keep the moon.

**Cost:** the lamps' shadow rays (the village's 325 lamps at 22:00 are the measure); the load's
split of the emissive sections (connected pieces by welded corners, as `open_edges`).

**Verification:** the Night view against Falcor's (light on the café's front and the street
within 0.8 to 1.25), the still-camera flicker under 0.2 %, the village's lamps unchanged.

**The prototype (2026-10-30, `FORGE_BISTRO_LAMPS=1`, the Bistro alone, off by default):** lamps
take a tint (`Lamp::tint`); the Bistro's 160 glowing pieces make 132 lamps (a lantern's inner
shell, facing in, left out: Falcor's emitters shine from their front alone), their rows
`RenderLayer::lamp`. Its street lanterns, glazed round their sides under a cap, light to 30°
above the horizon and not straight up (`Emission::LANTERN`): lit every way, they lit their own
brackets. At the Night view, Forge's light over Falcor's:
- **Light:** from 0.04–0.3 to 0.7–1.3 with the 64 nearest (the street 0.7–0.9, the far left
  street dark past them); with all 132 and the lanterns' cone, the street 0.97–1.08, the fronts
  1.0–1.5.
- **Flicker:** 261,857 pixels (18 %) to 4,230 (0.29 %) with the 64, 6,217 (0.43 %) with all;
  unchanged without the probes or the mirror rays: the resolve's choice of the lamps it traces,
  and the thin geometry (0.37 % by day). Short of 0.2 %.
- **Cost:** 4.1 ms of GPU to 8.6 (the 64) or 11.2 (all): no regions, so every pixel takes most
  lamps (D-065's lists, its clustered lists still to come). With the probes a metre apart (the
  authored scenes' look since), 13.6 ms with all: each probe ray's hit weighs every lamp
  (`hit_lamps`); the light and the flicker (6,111 pixels) the same.
- **Open:** the many-lights path's cost and its noise as lamps grow; the street lamps by ReSTIR
  DI not tried (`--street-restir` needs them `outside`).
- **Amended (2026-10-31, D-083's calibration):**
  - **No glow counted twice:** the bounce's, the port's, SHaRC's and the resolve's reference's rays
    leave a lamp row's glow out, as the probes' do (`ray_hit_lit`); they added it on top of its
    lamp.
  - **The Cornell box's light** (`--lab cornell`, an emitter of factors alone) is a glowing piece
    too: its emitters outside the Bistro glow in cd/m² at the scene's unit.
  - **Measured on the box:** Forge's path 0.025 from Falcor, the techniques 0.26 to 0.32. A flat
    piece's lamp gives the walls facing the camera half of Falcor's direct light: its shadow
    disc faces the surface and rises into the ceiling, and its smoothstep profile is half a
    cosine near the horizon (#369, to plan with #355).
  - **A planar piece's lamp lies on its face (#370, the same night):** Bitterli's bedroom read
    ×6 Falcor's light, its two panels behind curtains, because a flat piece's lamp was a disc
    centred half its radius in front of it and its shadow rays ended its radius short: the
    bedroom's 0.79 m disc stood 39 cm into the room, past the curtains, unshadowed.
    - **`Emission::Flat`:** a planar piece (its normals' sum over 0.98 of its area) is Lambert's
      emitter, its light the cosine from its axis (the same lumens as the smoothstep's). Its
      lamp sits 1 cm in front of its middle; its disc lies in its own plane, and
      its shadow rays end 2 cm short of the point they aim at (`lamp_ray`), so what covers it
      shades it. The rays' hits aim at a point of its disc, not its middle (`hit_lamps`).
      Pieces that bulge (0.5 to 0.98: a spotlight's glass) keep the old disc in front.
    - **Measured** (the techniques from Falcor, before → after): the Cornell box, the probes
      0.322 → 0.243, the port 0.255 → 0.125, SHaRC 0.261 → 0.132, its back wall matched; the
      kitchen's clay 1.14 to 1.17 → 0.10 to 0.15 (Forge's own path 0.110); the Bistro at night
      unchanged (0.549 → 0.544: its lamps are lanterns and globes).
    - **The bedroom** went from ×6 to −1.2 stops (SHaRC; the probes −1.6), and Forge's path with
      the lamps on is itself a stop under it with them off (the box 0.044, the kitchen 0.065):
      the lamp, not the caches. Two approximations of a large rectangle remain. Its disc of
      equal area misses the panel's top and bottom 12 cm, where the light escapes over and
      under the curtains. Its unshadowed light is taken from its middle, so the surfaces right
      beside its edge (the window's recess, the curtain's back) get about a third of theirs.
      The box's tall block and the tops of its walls show the same in small. Next: a flat
      piece as its rectangle, with its exact diffuse light (#369).
  - **A planar piece's lamp is its rectangle (#369, the same night):**
    - **The rectangle:** the least one round the piece's corners (`least_rectangle`: a side
      along an edge of their convex hull), where the piece fills 90 % of it; else the square of
      its area. `Emission::Flat` carries its turn and half sizes (`GpuLamp::beam_edge.yzw`, the
      96 bytes kept).
    - **Its shadow rays** aim at points uniform on it (the penumbra filter's stratified disc
      points taken back to the square by the concentric map undone, `disc_to_square`).
    - **Its unshadowed diffuse light is exact:** Lambert's formula for a polygon, the rectangle
      clipped at the surface's horizon first (`lamp_rect_share`), in the resolve, the penumbra
      filter's light and the hits' (`lamp_weight`). It is bounded by π times the luminance; the
      probes' hits still hold it under a lamp's of its intensity at their least distance
      (#294's halo). The highlight keeps the disc's lobe from the middle.
    - **The points behind a surface's horizon** (where its plane cuts the rectangle: the box's
      tall block) light nothing there, and their rays met the surface itself: the block took
      0.6 of its light. The penumbra filter's rays leave them out of both the shadowed and the
      unshadowed light (`lamp_look`); one ray (a hit's, the unfiltered path) aims at the
      point's mirror through the middle (`lamp_rect_target`). Counted as seen instead, the
      bedroom's bed (the panels' lower part behind its horizon, the rest behind the curtains)
      took a share of their whole light: the lamp model a third of a stop bright.
    - **Measured** (the review's fixes in): Forge's path with the lamps on against itself with
      them off (the lamp model alone) is the box's 0.044 → 0.021, the kitchen's 0.065 → 0.073
      and **the bedroom's 0.984 → 0.097**. From Falcor (after the flat lamp → after the
      rectangle):

      | | the probes | the port | SHaRC |
      |---|---|---|---|
      | Cornell box | 0.243 → 0.207 | 0.125 → 0.092 | 0.132 → 0.096 |
      | Bedroom, clay | 1.579 → 0.962 | 1.173 → 0.754 (#371) | 1.148 → 0.167 |
      | Bedroom | 1.751 → 1.191 | 1.064 → 0.596 (#371) | 1.320 → 0.445 |
      | Kitchen, clay | 0.146 → 0.167 | 0.102 → 0.122 | 0.114 → 0.106 |
      | Kitchen | 0.329 → 0.354 | 0.250 → 0.191 | 0.274 → 0.232 |
      | Bistro at night | 0.544 → 0.544 | 0.543 → 0.543 | 0.543 → 0.543 |

    - **What is left is the techniques':** the bedroom's light comes out of a 27 cm gap behind
      the curtains after many bounces. SHaRC follows it (0.177 from Forge's path in clay; the
      same each run); the probes, a metre apart, do not (a stop dark). The port is unstable
      there: two runs 0.16 to 0.27 apart (#371). Textured, the converter's materials add their
      gap (Forge's own path 0.392 from Falcor).
  - **The Bistro at night, the same measure (#372):** its lamp model alone is 0.309 from the
    glow (signed −0.21) with the 64 nearest lamps (D-065's lists), the far left street and the
    arch black; with all 132 (`--all-lamps`) 0.203, signed +0.17: bright round the lanterns,
    whose `Emission::LANTERN` sends their side's whole intensity straight down where their
    glass seen from below is 0.32 of it. The two half cancel in the default; the lanterns'
    profile waits for the count, a many-lights choice (D-065) for the owner.
  - **The night's reference was dark, not the lamps bright (#374, 2026-10-31):** the captures
    went through DLAA, whose history clamps a path's sparse bright hits, the glow's at night.
    Against Falcor's night, Forge's path read 0.822 of its light with DLAA and 0.997 without
    (`r.aa=off`, four frames 0.997 each). The lamp model (every lamp, Forge's path lit by them)
    against the glow traced without DLAA: the frame 1.007, the tiles' mean ratio 1.046, median
    1.029, where with DLAA it read 1.127 (#374's 13 %). What is left is local: the far left
    street's walls 1.5 to 1.7, the ground at the bottom right 0.75 to 0.8. With the lamps on,
    DLAA leaves the surfaces as they are (median 0.999) and dims the lanterns' glass seen
    directly, so #372's comparisons (lamps-on path against the lists) hold. `tools/gi-compare.sh`
    now captures without anti-aliasing (its timings keep DLAA); the night view again: the
    default (the lists, every lamp) 0.096 from Forge's path (×1.005) and 0.302 from Falcor's
    (×1.002), where with DLAA the 64 nearest read 0.317 (×0.967) and 0.544 (×0.795).
  - **Every view again without anti-aliasing (2026-10-31, at 38d140fc):** the morning's DLAA
    captures (the baseline's, the rooms' last, the night's every lamp) measured again against
    today's references (Falcor's clay was rendered again at 05:37, its albedo 0.496), then
    → the captures without it; the mean |log2| of the 144 tiles:

    | View | from Forge's path: probes, port, SHaRC, auto | from Falcor: Forge's path, probes, port, SHaRC, auto |
    |---|---|---|
    | Bistro Exterior_1, clay | 0.028 → 0.026, 0.046 → 0.058, 0.049 → 0.058, 0.026 | 0.180 → 0.161, 0.188 → 0.158, 0.137 → 0.108, 0.134 → 0.105, 0.158 |
    | Bistro Exterior_2, clay | 0.136 → 0.133, 0.087 → 0.093, 0.101 → 0.104, 0.133 | 0.216 → 0.206, 0.336 → 0.320, 0.170 → 0.153, 0.153 → 0.143, 0.320 |
    | Bistro Exterior_1 | 0.047 → 0.051, 0.059 → 0.062, 0.058 → 0.064, 0.051 | 0.080 → 0.082, 0.089 → 0.092, 0.096 → 0.099, 0.096 → 0.099, 0.092 |
    | Bistro Exterior_2 | 0.085 → 0.089, 0.117 → 0.116, 0.121 → 0.124, 0.089 | 0.387 → 0.378, 0.443 → 0.434, 0.480 → 0.466, 0.484 → 0.474, 0.434 |
    | Bistro at night (every lamp) | 0.187 → 0.096, all four 0.096 | 0.317 → 0.305, 0.428 → 0.302, all four 0.302 |
    | Classroom, clay | 0.368 → 0.360, 0.110 → 0.103, 0.110 → 0.117, 0.117 | 0.114 → 0.109, 0.454 → 0.437, 0.088 → 0.091, 0.091 → 0.096, 0.096 |
    | Classroom | 0.320 → 0.307, 0.096 → 0.101, 0.089 → 0.101, 0.101 | 0.151 → 0.164, 0.405 → 0.411, 0.150 → 0.142, 0.149 → 0.148, 0.148 |
    | Material chart | 0.031 → 0.033, 0.027 → 0.030, 0.028 → 0.030, 0.033 | 0.066 → 0.060, 0.088 → 0.083, 0.084 → 0.080, 0.085 → 0.081, 0.083 |
    | Cornell box (night) | 0.167 → 0.226, 0.074 → 0.105, 0.078 → 0.113, 0.113 | 0.025 → 0.033, 0.207 → 0.216, 0.092 → 0.095, 0.096 → 0.103, 0.103 |
    | Bedroom, clay (night) | 0.993 → 0.789, 0.712 → 0.281, 0.177 → 0.189, 0.189 | 0.194 → 0.110, 0.821 → 0.853, 0.865 → 0.344, 0.140 → 0.238, 0.238 |
    | Bedroom (night) | 0.981 → 0.988, 0.414 → 0.308, 0.271 → 0.282, 0.282 | 0.392 → 0.384, 1.191 → 1.190, 0.596 → 0.441, 0.445 → 0.440, 0.440 |
    | Kitchen, clay (night) | 0.220 → 0.136, 0.088 → 0.091, 0.080 → 0.083, 0.083 | 0.204 → 0.068, 0.112 → 0.132, 0.196 → 0.118, 0.161 → 0.097, 0.097 |
    | Kitchen (night) | 0.262 → 0.265, 0.136 → 0.127, 0.128 → 0.131, 0.131 | 0.206 → 0.203, 0.354 → 0.351, 0.191 → 0.201, 0.232 → 0.231, 0.231 |
    | Village landing | 0.507 → 0.486, 0.230 → 0.126, 0.110 → 0.122, 0.114 | |
    | Village room | 0.147 → 0.142, 0.052 → 0.057, 0.052 → 0.057, 0.074 | |
    | Village street | 0.116 → 0.112, 0.115 → 0.111, 0.116 → 0.111, 0.112 | |

    - **By day DLAA moved little** (±0.03 at most; the same capture with and without it within
      2 %, the chart 0.5 %). Forge's path over Falcor in the clay Bistro stands (×1.10, #373).
    - **The nights moved:** the Bistro's path ×0.823 → ×0.997 of Falcor's; the clay bedroom and
      kitchen 0.194 → 0.110 and 0.204 → 0.068 (×1.12 → ×0.96 and ×1.10 → ×1.00; the clay's
      albedo changed in between too). The rooms' columns between the morning and now also hold
      the day's lamp changes (the flat lamps, the port's), not DLAA alone.
    - **D-090's choice holds:** indoors SHaRC (and `auto`) 2 to 4 times nearer Forge's path than
      the probes (the kitchen 0.083 against 0.136, the landing 0.122 against 0.486), outdoors
      `auto` the probes' (Exterior_1 0.026, SHaRC 0.058); the village's room under `auto` 0.074,
      its hits past the door and windows the probes'.

## D-088 — The light in shade: a near field traced per pixel over the probes ✅ (proposed and decided 2026-10-30, the owner following the recommendations; after D-086; D-083)

**Why:** the largest error left in the Bistro by day, in every view. Measured in clay (every
material 50 % grey, no maps but the normals, nothing cut out: `FORGE_BISTRO_CLAY=1` and the
Falcor script's `BISTRO_CLAY`, Falcor's diffuse switched to Lambert as Forge's and 16 bounces),
Forge's light over Falcor's:
- **Exterior_1:** the sunlit street 1.1 to 1.2, the shade under the awnings and along the fronts
  1.3 to 1.8. Not the sun or the sky: the photograph lights a level surface with 4.134 of its
  units (the sun's 3° 2.173, the rest 1.961), Forge's sun and sky with 4.083 (0.99). The street's
  excess is the light the sunlit fronts send it, which the probes carry too.
- **Exterior_2:** the terrace's floor 0.8 to 1.0; the café's window bays 2 to 3, the probes'
  light in blotches on the panes where Falcor's are an even dark grey.
- **With the materials:** under Exterior_2's awnings the undersides 3 to 6 with the probes 4 m
  apart, 1.5 to 2 at 1 m (the authored scenes' look since); a mirror ray's hit in a room behind
  curtains lit as the street (the reflection the owner pointed at, 2026-10-30).

The probes' light is interpolated between points 1 to 4 m apart: a surface in a recess (a window
bay 30 cm deep, under an awning 2 m deep) takes the light of probes that see the open street.
Measured fixes within the probes fall short: 1 m over the whole view 8.6 ms and still 1.3 to 2.0
(D-086's note); GTAO at 5 m darkens the open ground as much as the shade (0.3 to 0.6).

**The proposal** (D-008's T2 tier's first step, the probes as its radiance cache):
- **The near field traced per pixel:** at half resolution, one cosine-weighted ray a pixel a frame
  to a range of twice the finest probe spacing (2 m at 1 m). A ray that leaves the range takes
  the probes' light in its direction (the far field, as now); one that meets a surface takes that
  surface's light: the sun through a shadow ray, the lamps (`hit_lamps`), and the probes' light
  at the hit for its own bounce. The probes no longer light what the near field sees closed.
- **Denoised by NRD** (D-049's, already in Forge for SIGMA): its diffuse denoiser (REBLUR, or
  RELAX for the flicker), temporal then spatial, before DLAA.
- **Mirror rays' hits** take the probes as now (one hit in many pixels); the near field is the
  primary surfaces'.
- **Rooms** (D-078) keep their grids; the near field corrects within them too.

**Targets:** in clay, the shade at both views within 0.8 to 1.25 of Falcor's; the textured views
after; a still camera's frames no less steady than now (0.37 % by day, 0.29 % at night); at most
1.5 ms at 1600 × 900 on the RTX 5070 Ti; the village's walk (`tools/map-check.sh`) and the city's
sentinels looked at for change.

**Considered:**
- **Probes everywhere 1 m apart:** 8.6 ms and 464 MB, the shade still 1.3 to 2.0.
- **Screen-space GI:** cheap, but blind to what the frame does not show (the awning above the
  frame's top), and it changes as the camera turns.
- **ReSTIR GI** (Ouyang et al. 2021): more light per ray, heavier; NVIDIA holds applications on
  ReSTIR's resampling (to check, as RTXDI's were): a later tier.
- **Probes placed by the geometry:** a redesign, and detail under their spacing still leaks.

**Patents:** a short-range ray gather over a radiance cache is decades old (final gathering);
Epic's Lumen combines screen probes and its own caches in ways to check before borrowing any of
its specifics; NRD under NVIDIA's RTX SDK licence, as SIGMA. To be listed per technique before
code, as the owner asks.

**Cost:** a new pass (rays and their hits' shading at half resolution) and NRD's diffuse
denoiser; measured before and after on the Bistro, the village and the city.

**First measurements (2026-10-30), before building the pass:**
- **The design by brute force** (`r.gi.reference=1`: 32 rays a pixel to 2 m, no denoiser,
  39 ms): in clay at Exterior_2 against Falcor in Lambert, the café's window bays from 2–3 to
  1.4–2.7, but other shaded tiles brighter (1.0–1.3 to 1.2–1.8). Every probe lookup near the
  awnings is too bright, at the rays' ends and at their hits alike: the probes' light itself leaks
  there, which a near field over the probes cannot take out. The design needs the leaks found
  first.
- **One leak found:** a probe's ray kept a quarter of every cut-out's triangles (crowns let light
  through), so the probes saw the sun through three quarters of the Bistro's awnings (their fabric
  a cut-out). Now only foliage rows; the other cut-outs take their alpha test whole, as the sun's
  rays. The probes' rays cost the same (1.84 ms at Exterior_2). The village's railings and other
  non-foliage cut-outs change too: to look at in its walk before a push.
- **Exterior_2 textured, against Falcor in Lambert** (rendered for it): the near terrace and table
  0.3–0.9 before the fix, 0.24–0.74 after, the café's window reflection 2.2–3.0: too dark near,
  too bright in the glass, where in clay the terrace matched (0.8–1.0). With the materials the
  terrace's light differs: the wood's grazing reflection of the warm street in Falcor, bluish in
  Forge, is the next to measure.
- **Matte** (`FORGE_BISTRO_MATTE=1` and the Falcor script's `BISTRO_MATTE`: base colours and normal
  maps kept, no "Specular" map, roughness 1, no metal): the near terrace and table still 0.23–0.8,
  so it is the diffuse light, not the reflection; with GTAO off 0.46–0.96, the near table top
  still about 0.5. In clay (every albedo 0.5) the same places matched: with the real, darker
  albedos round it, the light under the awning falls short, part GTAO's multi-bounce darkening of
  dark surfaces, part the probes there. Next: the probes under the awning (their placement,
  activity and what their rays meet), before the near field's pass.

**Measured further (2026-10-30, the owner: "continue with D-088"), and the design amended:**
- **Forge's own reference** (`r.gi.reference`, 32 rays a pixel, no denoiser): `4` a diffuse path
  without the probes, each hit lit by the sun and the lamps, the sky's light where a ray meets
  nothing; `2` one bounce traced to the scene's end and the probes' light at its hit; `3` two;
  `1` the 2 m near field above. Against Falcor in Lambert, in clay, the path is within 1.0 to
  1.15 at both views. A few tiles differ by 1.4 to 1.7 in every Forge picture alike, so they
  are the scenes' own differences. The scene, the sky and the units agree, so the light's
  system can be judged against Forge's path, the same scene, apart from what differs between
  the scenes.
- **The scenes' differences, apart:** with the materials (`FORGE_BISTRO_MATTE=diffuse` and the
  Falcor script's `BISTRO_DIFFUSE`: no reflection in either), Forge's path against Falcor:
  - under Exterior_2's awnings, the near table 0.77, the awnings' undersides 0.69, the café's
    front 0.79;
  - the street and the floor 0.98 to 0.99;
  - the café's front shows thin lines of sun, the converter's seams.
  The hits' missing highlight explains 3 to 6 %. The rest is a question of the scenes, listed
  for its own look.
- **The probes alone, against Forge's path (diffuse, Exterior_2):** the near table 0.60, the
  next 0.77, the floor 0.86, the café's front 1.27, the awnings' undersides 1.99, the window
  1.10. Three causes:
  1. **Coverage:** cascade 0 is 24 × 12 × 24 probes a metre apart, ±11 m round the camera.
     From Exterior_1's camera, 6.9 m up, the shaded fronts take the 4 m cascade and further
     back the 8 and 16 m ones (a debug colour per cascade). The 1 m cascade over the whole
     view (`--probe-finest 96,16,96`) brings Exterior_1's shade from 1.3–1.8 to 1.0–1.25 of
     Falcor's, in clay, now that the awning's leak is gone (1.3–2.0 with it).
  2. **The back-face floor:** DDGI's weight `facing² + 0.2` lets a probe behind the surface
     count a fifth. The probes inside the café, behind its panes, bleed through them where
     Chebyshev passes. Without the 0.2, the window bays go from 2–3 to 1.3–1.7 of Falcor's
     (the path: 1.15–1.3), and Exterior_2's clay from 0.8–3.1 to 0.9–1.4.
  3. **Irradiance, not radiance:** a probe holds a cosine-weighted mean of the light round a
     direction, so a ray ending in mid-air takes a blur: a bright street seen through a narrow
     opening thins out. With matte materials, the near field to 2 m lifts the near table from
     0.47 to 0.57 of Falcor's, to 30 m 0.87.
- **The probes' light at a ray's hit is biased bright:** the probes float in open air, see more
  sky than a surface does, and the far hits take the coarse cascades. With one bounce traced
  and the probes at its hit (`2`), clay comes out at 1.05–2.3 of Falcor's at Exterior_2 (the
  path 1.0–1.15), even without the floor. With two bounces traced (`3`), 1.03–1.6. With the
  sun alone at the hits, 0.1–0.6: in clay the bounces after the first carry most of the light
  under the terrace. With the Bistro's real albedos (0.03 to 0.4) they carry little: one bounce
  traced lands within 1.0–1.10 of the path everywhere measured (the near table 1.04, the
  awnings 1.10, the café's front 1.05), against the probes' 0.60–1.99.
- **The 2 m near field cannot work over these probes:** its rays' ends read irradiance as if it
  were radiance (3 above), and its hits read the bias above.

**The amended design** (still D-008's T2 tier's first step; the decision's other parts stand):
1. **One bounce traced per pixel to the scene's end,** not to 2 m: at half resolution, one
   cosine-weighted ray a pixel a frame. Its hit is lit by the sun through a shadow ray, the
   lamps (`hit_lamps`), its emission, and its albedo times the probes' light for the bounces
   after. A ray that meets nothing takes the sky's radiance along it. GTAO is off where the
   bounce is on: its rays see the occlusion.
2. **Denoised** by NRD's REBLUR as decided, with Forge's own temporal and spatial filter where
   NRD is absent.
3. **The probes:** the back-face floor goes (2 above). They stay the light past the traced
   bounce, the mirror rays' hits and the far field. Their coverage (1 above) matters less
   under the traced bounce; it is measured again once the pass is in.
4. **Brighter scenes:** the bias at the hits shows there, and the village measures it. Its
   whitewashed rooms (`city-plan --village 600`, the map check's "room" and "landing" at 0.75,
   against `r.gi.reference=4`; the reference's rays pass the houses' clear glass as the probes'
   do):
   - the probes alone light the room's walls at 1.4 to 1.7 and its table's corner at 2.0, the
     landing at 1.1 to 2.4. The room reads green with the lawn's light, where the path's is
     warm with the sun's and the wood's;
   - one traced bounce brings the room to 1.1 to 1.5, two to 1.1 to 1.3: with an albedo near
     0.8, a bounce's bias compounds bounce after bounce;
   - DDGI's back-face floor changes these rooms by 1 to 5 %.
   The remedy is a cache on the surfaces themselves, its rays leaving the surface's own side,
   read at the hits. Ward's irradiance caching (1988) is free; NVIDIA's spatial-hash radiance
   cache and path space filtering may be patented. That cache is a proposal of its own, its
   patents researched first; the traced bounce comes before it.
5. **Targets, against Forge's own path (`r.gi.reference=4`):** 0.9 to 1.1 with the materials,
   0.85 to 1.2 in clay; a still camera no less steady than now; at most 1.5 ms at
   1600 × 900.

**Built (2026-10-30): the traced bounce** (`r.gi.trace`, on by default since the owner's word that
day, with the finest cascade ahead below):
- **`gi/trace`:** one cosine-weighted ray a pixel to the scene's end, past a house's clear glass.
  Its hit takes the sun through a shadow ray, the lamps, its glow, and its albedo times the
  probes' light; a miss takes the sky's light along the ray. Rays start 2 cm off along the
  vertices' normal and are mirrored over their triangle's own face where that normal bends round
  a box's edge. A terrain's start as its sun's rays (`shadow_bias`), past the gap between its
  drawn cut and the rays' (#364): from 2 cm the island's sea floor, drawn up to a metre under the
  cut, met the cut's backs in black wedges that moved with the view. Elsewhere a ray that meets
  the back of its own mesh's cut within the gap that cut may leave (twice its error and the drawn
  cluster's) traces on past it (#367): the city's towers, 3 M triangles cut to 40,000, stand up
  to 1.4 m off their drawn walls, and the rays from their recessed windows and under their ledges
  met the cut's back in black wedges (the owner, 2026-10-30); a start past such a gap would skip
  what really stands close. In the village, whose two meshes over the budget are cut at 0.45 and
  2.33 m, the walls along a stair's treads took their light back (the landing's view +0.3 EV
  there, +0.03 over the frame). Where the cut fills the wedge under a ledge, the wall's rays up
  from under it now find the sky rather than black. The light is written in YCoCg with the ray's normalised length, as REBLUR reads
  it.
- **Denoised by REBLUR_DIFFUSE** (NRD 4.18, the library SIGMA comes from; `forge_gpu::nrd` now
  runs either from one instance type). Half the pixels traced each frame, in a checkerboard (every
  pixel since the landing's fix, below), and REBLUR reuses the guides the sun's shadow rays write for SIGMA. Without NRD the resolve reads
  every pixel's ray as traced, noisy under DLAA; the decision's own fallback filter is still to
  write.
- **The review's fixes** (a read-only review of the batch):
  - The standard resolve pass now declares the bounce's image.
  - NRD's first-frame clears leave its inputs alone: REBLUR lists the motion vectors, which a
    clear wiped for a frame.
  - The bounce's lamp seed no longer cancels the frame's own term.
  - REBLUR's history starts over after a frame it did not denoise.
  - The pass runs only with a ray scene.
  - The light and lengths are sanitised as NRD's front end does it.
  - The 16-bit index is checked in release builds.
  - The path reference runs without probes.
  - Not taken: directions turned beyond TAA's cycle. A pixel then meets more than 4 directions,
    but a still camera's changing pixels doubled (5,054 to 10,507). The directions only move off
    the sun's disc points.
- **Found on the way:**
  - **A race.** The bounce's image shares memory with the lamp shadow rays' images. In the
    village's rooms on the RTX 5070 Ti, REBLUR read texels of the latter back: a bright band at
    the frame's foot. The serial frame and separate memory hid it. REBLUR and `gi/trace` now
    stand behind full barriers (#349's kind), about 0.3 ms.
  - **Black seams.** With the sun's 15 cm ray bias, a wall's rays behind a table started inside
    its top and legs; with the lamps' few millimetres, the points of faces seen edge on still
    did. Both gave black seams under REBLUR, which the owner saw. 2 cm along the normal, as the
    reference, has none.
- **Measured** (Falcor in Lambert at 16 bounces, the textured scene):
  - **Exterior_1:** 0.86–1.12, but for the tiles every Forge picture shares (the probes alone
    1.0–1.75). D-088's first target is met there.
  - **Exterior_2:** the terrace 0.35–0.9 (the probes 0.24–0.8). In diffuse (no reflection on
    either side) it lands on Forge's path, whose 0.77–0.99 there are the scenes' own differences.
    The rest is the surfaces' reflection, D-086's side.
  - **Diffuse Exterior_2, against Forge's path:** every region within 1.0–1.1.
  - **The village's whitewashed room, against Forge's path:** 1.1–1.4 (the probes 1.15–1.7). This
    is the probes' bias at the hits, for the surface cache below.
  - **A still camera:** 5,054 and 2,705 changing pixels at Exterior_1 and _2 (5,103 and 3,187
    before).
  - **Cost** at 1600 × 900 on the RTX 5070 Ti: `gi/trace` 0.8–1.0 ms, REBLUR's passes 0.45, their
    barriers about 0.3. The frame +2.0 ms at Exterior_1, +1.7 at Exterior_2, +1.4 in the village's
    room: over the 1.5 ms target.
- **Next:**
  - The owner's look at the bounce in the Bistro and the village, before it is on by default.
  - The cost under 1.5 ms:
    - Under the bounce, 64 probe rays save 0.74 ms, the diffuse light unchanged within 2–5 %.
      But the café's glass reflections rise by 30 %: the mirror rays' hits read the probes.
    - GTAO is unused where the bounce is on (about 0.2 ms). Used again since, for the contacts
      (the landing, below).
    - The barriers can be narrowed.
  - The fallback filter without NRD.
  - **The finest cascade ahead of the camera** (`r.gi.probe_ahead`, the owner's question through a
    side session, 2026-10-30). Cascade 0 stands that many metres along the view instead of round
    the camera, the same probes, scrolling by whole cells. Against Forge's path in clay, the mean
    of |log2| of the 16 × 9 tiles' ratios:

    | | Exterior_1, probes | Exterior_1, bounce | Exterior_2, bounce |
    |---|---|---|---|
    | round the camera | 0.155 | 0.059 | 0.224 |
    | 10 m ahead | 0.150 | 0.057 | 0.216 |
    | 20 m ahead | 0.099 | 0.041 | 0.207 |
    | 1 m everywhere (`--probe-finest 96,16,96`) | 0.076 | 0.035 | 0.204 |

    20 m ahead costs nothing: 4.0 ms with the probes alone, against 11.6 for 1 m everywhere. It
    holds most of 1 m everywhere's gain at Exterior_1. At Exterior_2 the camera already stands
    among the shade. Turning at 5° a second (2 px a frame), the frames' changes against the next
    one shifted back are the same within 0.1 % (276,598 round the camera, 276,514 ahead, with the
    bounce), with no spike.
- **On by default (the owner, 2026-10-30: "turn on the bounce and probes ahead by default"):**
  - **The bounce needs REBLUR.** Where NRD or the sun's shadow rays' guides are missing, the
    probes light alone, as before; the rays are never read raw.
  - **The cascade goes only as far ahead as the view reaches.** A fixed 20 m made the village's
    room worse (0.230 to 0.272), the camera left outside its finest probes. `gi/view reach`
    (0.01 ms) takes the geometric mean of the distances a 32 × 18 grid of pixels shows, read
    back a few frames later. The cascade stands at the smaller of that and `r.gi.probe_ahead`,
    20 m by default, moved in steps of 2 m so a still camera's cascade never flips across a
    cell's edge. Exterior_1 0.042, Exterior_2 0.218, the room 0.230: each as good as its best
    fixed choice.
  - **Leaves keep the probes' light.** Far from the camera the rays meet a crown's opaque proxy
    (D-075's shadow world), and a leaf's ray started inside its own: the near trees seen from
    the air went black beside their impostors. The trace writes the probes' light there for
    REBLUR. The bounce's rays keep a quarter of the crowns' leaves, as the probes' do.
  - **The village** (the map check against the morning's run): every view changes, ꟻLIP
    0.07–0.13. The paths under the trees are darker, the rooms closer to the path. Each frame
    costs 1.2–1.8 ms more. A still camera in the main street changes 5,737 pixels, against
    5,311 without the bounce. The accepted captures change everywhere: to accept with the push.
  - **The surface cache** (`research/surface-radiance-cache.md`). No design was found clearly
    free of live patents: NVIDIA's Pantaleoni family runs to 2039 and Keller's path space
    filtering to 2034, both broad. The note recommends persistent records on a dense clipmap, no
    hashing. It needs the owner's decision, and counsel's view, before any code.

**The probes' budget under the bounce, and the landing (2026-10-30, the owner: "continue with the
probes' budget under the bounce"):**
- **The landing.** The owner, on the bounce's village views (through the village's side session):
  "looks better except for landing.png, the walls and stair rails are not smooth anymore". Two causes, two fixes:
  - **Grain.** The stairwell takes its light through one opening, and half the pixels a frame
    left REBLUR's grain on its walls and rails. A longer history, a narrower anti-lag and a wider
    blur only thinned it. Every pixel traced takes it away: `gi/trace` 1.09 → 2.16 ms at
    Exterior_1, the frame +0.95 ms there and +0.8 at Exterior_2. A still camera on the landing
    changes 20 pixels per 10 frames, 34 before.
  - **The steps' creases.** REBLUR blurs the contacts the rays see, a stair's steps into one
    slope. GTAO stays under the bounce at a shorter reach (`r.ao.contact`, 0.6 m; `r.ao.radius`,
    1.5 m, elsewhere), with the material's occlusion map and the reflections' occlusion as under
    the probes. The references (`r.gi.reference`) still take none. Against Forge's path, the mean
    |log2| of the 16 × 9 tiles:

    | | Exterior_1, clay | Exterior_2, clay | the village's room |
    |---|---|---|---|
    | the bounce as it was | 0.041 | 0.219 | 0.157 |
    | every pixel, GTAO at 0.3 m | 0.039 | 0.175 | 0.159 |
    | at 0.45 m | 0.033 | 0.159 | 0.152 |
    | at 0.6 m | 0.033 | 0.151 | 0.144 |
    | as built: 0.6 m, the Bistro's probes 2 m apart | 0.029 | 0.118 | 0.144 |

    At 1.5 m, over half the pixels, Exterior_1 went to 0.068: GTAO over the occlusion the rays
    already see. The textured Exterior_2 against the bounce as it was: the café's window 0.997,
    the terrace's floor 0.94, the near table 1.01.
  - **Still too bright:** the landing's walls and steps are 1.5 to 1.6 times the path's, as
    before. That is the probes' bias at the hits, for the surface cache.
  - **The leaves** keep the probes' light, now under GTAO at 0.6 m rather than 1.5 (the frame has
    one radius). The map check's crowns look unchanged.
- **The probes' cost under the bounce.** Their rays and blend take 2.92 ms at Exterior_1, 2.44 at
  Exterior_2, about 2.1 in the village's room. The levers, as the frame's GPU ms, one run each
  (the bounce as it was: 6.43 / 5.71):

  | Lever | Exterior_1 | Exterior_2 | The light |
  |---|---|---|---|
  | 64 rays a probe | 5.71 | 4.97 | the café's window +24 % |
  | 96 rays | 6.06 | 5.39 | the window +6 % |
  | an update every 4 frames | 5.35 | 4.66 | rooms 4 to 6 % brighter |
  | the finest cascade 16 × 8 × 16 | 5.58 | 4.94 | further from the path: 0.059 / 0.238 |
  | 5 cascades | no saving | | |
  | probes 2 m apart | 5.87 | 5.13 | nearer the path in clay: 0.040 / 0.185 |
  | 2 m and every 4 frames | 4.96 | 4.32 | |

  - **Why the light shifts.** Fewer rays leave a probe's distance moments noisier, so Chebyshev's
    test leaks more where light comes through openings. Updating every few frames brightens
    rooms. Cadence 1 stays, the owner's choice for stillness (2026-10-16).
  - **Taken: the authored scenes' probes 2 m apart.** The Bistro's awning is 1.08 of Falcor's
    either way.
  - **The village keeps 1 m.** 2 m saves only 0.17 ms there (the room 7.16 → 6.99 ms, 0.157 →
    0.153), and 1 m is what kept the sun from coming through a 3 m room's wall by its doorway.
    With an update every 4 frames as well, the room costs 6.43 ms but strays to 0.186.
  - **A latent fault fixed.** A room grid's probe (D-078) updates every frame at any cadence, but
    took the settled probes' hysteresis^cadence. At cadence 4 the village's room stood 0.220
    from the path; with each frame's own share, 0.179. At the default cadence of 1 nothing
    changes.
  - **REBLUR stays at NRD 4.18's defaults.**
  - **A still camera** is as steady with the levers: Exterior_1 changes 5,045 pixels, 5,090 with
    2 m and every 4 frames; the village's street 5,822 and 5,996.
- **As built, against the bounce as it was** (the frame's GPU ms, 1600 × 900, the RTX 5070 Ti):
  - Exterior_1: 6.43 → 6.88.
  - Exterior_2: 5.71 → 5.90.
  - The village's room: 7.16 → 7.82.
  - Its street: 6.30 → 6.90.
  - The landing: 6.38 → 6.83.

  Exterior_1 is 2.4 ms over the probes alone a metre apart (4.49), still above the 1.5 ms
  target. The rest waits for the surface cache (`research/surface-radiance-cache.md`).

**The surface cache, decided (the owner, 2026-10-30, through the Falcor session, and confirmed in
this one: "yes, record it and download them"):**
- **What:** Bevy Solari's world cache with kajiya's dense camera-centred clipmap index in place of
  its hash. Both are MIT or Apache-2.0; Forge takes them under Apache-2.0 and keeps their notices
  (neither ships a NOTICE file).
  - **Bevy Solari:** Bevy 0.20.1, `crates/bevy_solari`.
  - **kajiya:** Embark's, archived, `assets/shaders/ircache/`.
- **From Solari:**
  - **Cells live while they are read.** Unread for ten frames, a cell is freed. The live cells
    are compacted each frame into a list.
  - **Each live cell's update:** its direct light, one cosine ray whose hit reads the cache
    itself (the bounces after), and the sky where the ray meets nothing.
  - **The blend:** a sample count that is capped, and cut back where the light changes.
  - **Cells grow with the distance from the camera,** and a query is jittered in the surface's
    plane.
- **From kajiya, the index:**
  - Twelve 32³ levels round the camera, each twice the last, scrolling with it.
  - Cells offset half a cell along the normal.
  - A cell's slot is its coordinate in its level: no hash, no collisions.

  Solari keys its cells by the normal too; kajiya's cells hold one entry each. How a dense cell
  keeps a wall's two faces apart is the design's first question (the research note's normal and
  plane tests).
- **Forge's own part:**
  - The cells' rays' hits are lit as the bounce's: the sun through a shadow ray, the lamps, the
    glow.
  - The bounce's hits read the cache in place of the probes.
  - The probes stay for volumes, movers, and the frames before a cell exists.
- **Patents** (`research/surface-radiance-cache.md`):
  - **The risk the owner accepts, without a counsel's opinion:** NVIDIA's Pantaleoni family
    (US 11,244,493, US 11,961,176, US 12,633,037) and Keller's path space filtering
    (US 9,953,457).
  - **Avoided by the design:**
    - **US 10,614,613** (quantise, hash, resolve collisions): Solari's cache is exactly that (a
      hashed key, a checksum, linear probing), so its index goes for kajiya's.
    - **US 11,790,594** (a cache used by a ray's length): Solari reads its cache only past √3
      cells (`initial_path.wesl`); Forge reads it at every hit.
    - **US 11,315,310's kind:** kajiya's per-voxel voting and its ReSTIR update are not taken.
- **Credits** in the commit that brings the code in:
  - `CREDITS.md`: a row each under "Libraries and tools" and "Techniques", and an entry each in
    "Giving back".
  - The ported files in `shaders/third-party/` (and the Rust beside them), each with a header
    naming its origin, the commit, the licence and Forge's changes.
- **The sources:** bevy_solari and kajiya's irradiance cache only, at Bevy's v0.20.1 (a0577b5c)
  and kajiya's 9fdec0f0. They sit in `surface-cache-ref/` beside the repository, as Falcor does,
  and are never committed.
- **Tests:** against `r.gi.reference=4` in the village's room, on its landing and in the Bistro's
  clay views; the cost; a still camera indoors, for flicker.

**The surface cache's design (2026-10-30, the owner: "start building the surface cache"):**
- **Entries:** a pool of 65,536, 80 bytes each (5.2 MB). Each holds:
  - the surface point that made it (its rays' start) and its normal;
  - its cell and its slot in the index;
  - its light (Forge's irradiance units, as the probes': sky and bounces, no direct sun) and the
    samples blended;
  - this frame's sample;
  - the frames it has left unread.

  Free entries wait on a stack; a slot takes one by compare-and-swap, and gives it back when the
  entry dies.
- **The index (kajiya's):** twelve levels of 32³ cells round the camera, the finest 15 cm (Solari's
  base cell), each level twice the last. Six slots a cell, one per facing (the normal's main axis
  and sign), so a wall's two faces never share an entry: 12 × 32³ × 6 slots of 4 bytes, 9.4 MB.
  - **The level:** the finest whose cells, less two at the edge, hold the point (kajiya's rule).
  - **The cell:** the point is jittered within the cell's size in its surface's plane (Solari's),
    then moved half a cell along its normal (kajiya's).
  - **Scrolling:** a level's cells wrap in place as the camera moves. An entry whose cell has
    left its level's window is freed before anything reads the index, so a slot always holds its
    own cell's entry: no tag, no hash, no collisions.
- **Each frame,** before the bounce, three passes over the pool:
  1. **`gi/cache age`:** frees the entries unread for ten frames (Solari's lifetime), or out of
     their level's window, or all of them after a reset.
  2. **`gi/cache rays`:** one cosine-weighted ray per live entry, from 2 cm off its point, in the
     bounce's world (the trees' cards near, their proxies far), not the probes': since #363 the
     probes' rays pass the houses' furniture, and the cache's rays in their world let the light
     in past it, the village's room 0.3 stop over the path. A hit is lit as the bounce's are (the sun through a shadow ray, the lamps, the glow) plus its
     albedo times the cache's light there (the bounces after). It falls back to the probes'
     light where the cache has no light yet, and on leaves, whose rays start inside their crowns'
     proxies (D-075). A miss takes the sky's light along the ray.
  3. **`gi/cache blend`:** the samples averaged, then once an entry holds 32 an exponential
     average weighing each new one 1/32: as long to follow a change as the probes' hysteresis.
     Solari's blend cuts the samples back where the luminance changes; with one ray an entry a
     frame that change is the rays' noise, and it took the landing's still camera from 20 changing
     pixels a frame pair to 319 (33 without it).
- **Reads:** the bounce's hits (`gi/trace`) and the cache's own rays' hits look the cache up,
  making an entry where none is. They read it unconditionally, whatever the ray's length (D-088's
  patents above). Where it has no light yet, the probes' light stands in. The pixels never read
  entries directly; mirror rays keep the probes for now. The cache's rays take the lamps as the
  probes' do (no street lamps, none nearer than 25 cm, #294, #254): one ray an entry a frame
  near a bulb is the probes' halo again.
- **Within a pass** (the review, 2026-10-30): an entry is written before its number is
  published, and a freed one is cleared, so a lookup that meets a number before its entry finds
  no light and takes the probes'. The rays' pass traces only the entries alive at `gi/cache age`.
  The blend counts the entries in use for the host (`SurfaceCache::most_used`, logged at exit).
- **Not handled:** the probes' rooms (D-078) keep a room's light from the street's; the cache's
  cells do not, and at a few metres from the camera a room's floor and the street outside, both
  facing up, can share an entry. Finer levels (64³) did not change the village's room (0.305
  against 0.298 before the rays' world was fixed).
- **On by default and taken back (2026-10-30):** the owner turned it on (9dc41903). Made by the
  first lookup to ask, as Solari's and kajiya's are, an entry's point and its rays' seed
  followed the threads' order: two identical runs of the classroom differed in 7.6 % of their
  pixels and the culling harness in up to 3,884, so on by default it needed the same bytes from
  run to run (D-016). 2039a0ac made each cell's entry at the point of its lookups' least hash,
  by a 64-bit atomic minimum, kept for life. That reads almost word for word on NVIDIA's
  US 12,394,144 B2 (consistent sampling for spatial hashing, to 2041-12-08, claims 1, 2, 5, 6 and
  7; `docs/research/surface-radiance-cache.md`, "The claims read"), which the research's table
  flagged ("no elected anchors") and whose claims were read only after the push. Both were
  reverted the same evening; the cache is off by default again and nondeterministic.
- **Amended: SHaRC replaces this port** (the owner, 2026-10-30, confirmed in the session that
  reverted it). NVIDIA's SHaRC (1.8.3, shader-only, the NVIDIA RTX SDKs licence; RTXGI 2.8.0
  beside it), used unmodified from the git-ignored `sharc-sdk/` and `rtxgi-sdk/` and compiled
  into Forge's shaders, lights the bounce's hits. Its patents (the hash map, US 10,614,613, and
  the families above) are NVIDIA's own, used as designed, as RTXDI covers ReSTIR (an implied
  licence, no written grant). Solari and kajiya (`surface-cache-ref/`, beside the repository)
  stay as reading and the fallback; this port stays behind `r.gi.cache`, off. Its checks:
  - the landing near the path tracer (×1.53 with the probes);
  - no light through the village's thin walls and floors (its cell size near the camera);
  - its passes' cost on the 5070 Ti;
  - the AMD iGPU (it needs native fp16 and ray tracing).

  The order, one theme at a time: RTXDI's ReSTIR DI for the lamps through NRD first, then
  SHaRC for the bounce's hits, measured on the landing and the Bistro, then RTXDI's ReSTIR GI
  fed by SHaRC only if noise or cost calls for it; never a ReSTIR of Forge's own
  (US 11,315,310). If the port is ever wanted on again, an entry whose point no election picks
  (an average of the claims, a point the cell fixes) would stay out of US 12,394,144's claim 1.
- **SHaRC under the bounce: the design** (2026-10-30, for the owner's review before code;
  `docs/research/sharc-and-nrc.md`):
  - **Three graph passes, SHaRC's own:**
    1. `gi/sharc update` (compute, ray queries): one pixel per 5 × 5 block, moving each frame by
       the noise frame, starts a path at its G-buffer surface. `path_from`'s loop (Lambert,
       cosine directions, `PATH_MOST_BOUNCES`) calls `SharcUpdateHit` at each vertex and
       `SharcUpdateMiss` with the sky on a miss. At each vertex:
       - the sun through its shadow ray as SHaRC's separate emissive term;
       - the lamps as its direct light (`hit_lamps`, as the probes' rays take them);
       - the albedo as the demodulation factor and the segment's throughput.
    2. `gi/sharc resolve` (compute, async queue): `SharcResolveEntry` for every slot; the sample's
       32 frames of accumulation and 64 to eviction to start.
    3. **The query, inside `gi/trace`:** at the bounce ray's hit, `SharcGetCachedRadiance` with
       the hit's own sunlight (its shadow ray, as today) and albedo. The hit's sun shadows stay
       exact; the voxel gives only the light after the hit.
       - Where the segment is shorter than the voxel's diagonal (SHaRC's rule), the bounce
         traces one more cosine segment, at most two, and reads there.
       - Where no voxel answers, the probes stand in, as for the port.
  - **Data:** the three buffers by device address, through a bridge header that sets SHaRC's
    `RW_STRUCTURED_BUFFER` and `BUFFER_AT_OFFSET` before its includes; the headers stay
    unmodified, included from `sharc-sdk/`.
    - Size: 2²⁰ slots to start (40 MiB), raised if the village or the Bistro fills more than a
      third.
    - Positions: the scene's frame (`to_scene`), the camera's position in the same frame. The
      cache is cleared on a reset, an origin shift or a scene change.
    - `sceneScale`: chosen so the voxels near the camera are thinner than the village's walls
      (50 and 100 tried).
    - `radianceScale`: chosen from the sun's radiance.
  - **Switch:** `r.gi.cache` takes a value: `sharc` (the default once it passes), `nrc`
    (NVIDIA, the trial), `port` (the Solari/kajiya port) and `off` (the probes at the hits).
  - **Fallbacks:**
    - Without `sharc-sdk/` (CI, a GitHub build), the shaders compile without SHaRC (a define set
      when its headers exist) and `sharc` falls back to the probes.
    - Without native fp16, the same.
    - Without 64-bit atomics, SHaRC's lock buffer.
  - **Checks, in order:**
    1. The A/B harness at 0 px and two runs byte-identical (SHaRC sums integers; its slots
       follow the threads, so this is checked, not assumed).
    2. The classroom against Falcor, sun and sky apart (the port: ×0.92 sun, ×0.98 sky).
    3. The landing near the path tracer (×1.53 with the probes).
    4. No light through the village's thin walls and floors.
    5. The passes' cost on the 5070 Ti, the F1 overlay and `PROFILE.md`.
    6. The AMD iGPU.
  - **NRC afterwards, on the same hook** (NVIDIA only, behind `nrc`):
    - The training paths come from the update pass's loop at NRC's training size; the query
      records are written at the bounce's hit.
    - `QueryAndTrain` runs as a graph pass recording NRC's commands.
    - A small C shim of Forge's own (`tools/fetch-nrc.sh` builds it against `NRC_Vulkan.lib` into
      `nrc-sdk/bin`) is loaded at run time from `forge-gpu`.
    - The device enables NRC's extensions only when its DLL loads.
    - It stays if it beats SHaRC against the path tracer at a cost worth it.
  - **Patents:** Forge's own code around SHaRC adds no cache mechanics; the hashing, the
    segment-length rule and the blend are SHaRC's, used as designed (the owner's SDK rule).
- **`r.gi.cache`,** off until the owner turns it on. Measured (2026-10-30), the mean |log2| of
  16 × 9 tiles against `r.gi.reference=4`, and a still camera's changing pixels per pair of
  frames 10 apart:

  | View | probes at the hits | the cache |
  |---|---|---|
  | The village's landing | 0.811 (×1.53) | 0.201 (×1.06) |
  | The village's room | 0.144 | 0.058 |
  | The Bistro's Exterior_1, clay | 0.029 | 0.047 (×0.98) |
  | The Bistro's Exterior_2, clay | 0.118 (×1.01) | 0.103 (×0.96) |
  | Still: landing, room, street | 20, 683, 5,694 | 21, 692, 5,704 |
  | Still: Exterior_1, Exterior_2 | 4,899, 2,863 | 4,852, 2,873 |

  The frame costs the same: the cache's three passes 0.02–0.17 ms, `gi/trace` 0.25–0.45 ms
  cheaper in the Bistro (its hits read one entry instead of the probes' eight). At most 25,701
  entries in the room, 55,081 at the Bistro's textured Exterior_2, of 65,536.
  Against Falcor in the classroom (`--lab classroom`, 2026-10-30; Falcor's path tracer with
  Lambert's diffuse, as Forge's, 16 bounces, 4,096 samples a pixel), the same measure and the
  frame's light:

  | Classroom | Forge's path | probes at the hits | the cache |
  |---|---|---|---|
  | Clay | 0.090 (×0.98) | 0.353 (×1.14) | 0.145 (×0.93) |
  | Clay, the sun alone | 0.106 (×0.98) | 0.279 (×1.05) | 0.166 (×0.91) |
  | Clay, the sky alone | 0.067 (×0.98) | 0.958 (×1.35) | 0.147 (×0.98) |
  | Textured | | | 0.151 (×0.98) |

  Falcor's default diffuse (Frostbite's, 1/1.51 at roughness 1) made every Forge image a stop
  brighter indoors, Forge's path as well: the gap grows with each bounce. The textured row
  measures the materials too (the coated whiteboard, the metal frames).
- **The hits' shadow rays start by their cut (2026-10-30, found in the classroom):** a ray's hit
  (the bounce's, the path's, the cache's and the probes' rays, the mirror rays) started its sun
  and lamp rays `SHADOW_BIAS`, 15 cm, along `n + sun`: from under a ceiling they started above
  it. The fittings, the screen's box and the walls' tops took the sun through the ceiling at
  every bounce, and the band under it came out 0.5 to 1 EV over Falcor's with the sun alone,
  Forge's path as well (with the sky alone it matched, 0.067). A hit lies on the cut the rays
  trace, so it now starts as a drawn surface does, twice its cut's error from 2 cm to 15 cm
  (`hit_shadow_start`; terrains and leaf cards keep 15 cm). Before, the classroom's clay: the
  path 0.184 (×1.08), the probes 0.416 (×1.19), the cache 0.127 (×1.005); textured, the cache
  0.203 (×1.08). The cache's near match had been the leak covering its own deficit, a few per
  cent dark, as in the Bistro (×0.96 to ×0.98).
- **The probes' sky leaks into the classroom (2026-10-30):** with the sky alone, the probes at the
  hits came out a stop over Falcor's in every tile (+0.96 EV on average), +1.12 with one bounce
  traced before them (`r.gi.reference 2`) and +0.66 with two, the path matching. Not the open
  sky's fill (renormalising in its place changed no tile). It is the probes' spacing: 2 m apart,
  the probes outside the room's thin walls and over its ceiling, in the open sky's light, weigh
  in the lookup inside; the room's own light is a few per cent of theirs. The authored scenes'
  probes (2 m, chosen at the Bistro's street) against Falcor, the sky alone and the whole clay:

  | Probes | Sky alone | Whole | GPU frame (probe rays + blend) |
  |---|---|---|---|
  | 2 m | 0.958 (×1.35) | 0.353 (×1.14) | 4.11 ms (1.33) |
  | 1 m | 0.214 (×1.04) | 0.132 (×0.98) | 4.67 ms (1.63) |
  | 0.5 m | 0.110 (×1.00) | 0.145 (×0.94) | 5.28 ms (2.13) |

  The cache matches the sky (×0.98) for less (3.86 ms); its deficit is in the sun's bounce
  (×0.91).
- **Credits:** the shaders carry Bevy's and kajiya's names, commits and licences in their header,
  and the changes Forge made.
- **SHaRC under the bounce: built** (2026-10-30, `r.gi.cache sharc`, off by default;
  `crates/forge-render/src/sharc.rs`, `shaders/sharc.slang`). The design above, with these
  changes:
  - **The lamps** go with the sun as SHaRC's separate emissive term (`hit_lamps`, as the probes'
    rays take them), not as its direct light. A voxel holds only the light that reaches it, and a
    hit's own sun and lamps stay exact where it is read.
  - **`SHARC_PROPAGATION_DEPTH` 4,** not SHaRC's 2. With 2, the landing's whitewashed walls
    (albedo near 0.9) took their many bounces from voxels that settle over hundreds of frames,
    and frame 300 was a third of a stop under the path tracer (0.376, ×0.85). With 4: 0.089
    (×0.99), for 0.03 ms more in the update. SHaRC's guide raises it where light needs longer
    paths.
  - **64-bit buffer atomics are required:** no lock-buffer fallback. Without them, without
    16-bit storage or without `sharc-sdk/`, `sharc` keeps the probes.
  - **Both passes on the main queue:** the update reads this frame's visibility buffer.
  - **The path's root** (the pixel's own surface) has demodulation 1 and throughput 1.
  - **The cache is emptied** when it is turned on again, not on a cut: its voxels are in the
    scene's frame, and nothing shifts that frame's origin or swaps the scene at run time yet.
  - **`nrc`** is not a value yet.

  **Against Forge's path tracer** (`r.gi.reference=4`; the mean |log2| of 16 × 9 tiles at frame
  280–300, and the frame's light):

  | View | the probes (default) | the port | SHaRC |
  |---|---|---|---|
  | The village's landing | 0.762 (×1.50) | 0.166 (×1.01) | 0.089 (×0.99) |
  | The village's room | 0.138 (×0.95) | 0.060 (×0.99) | 0.068 (×0.97) |
  | The Bistro's Exterior_1, clay | 0.029 (×1.00) | 0.043 (×0.98) | 0.048 (×0.98) |
  | The Bistro's Exterior_2, clay | 0.150 (×1.02) | 0.088 (×0.96) | 0.101 (×0.95) |

  The village's street, measured after with `tools/gi-compare.sh`: the probes and SHaRC alike,
  0.115 (×1.01). Its error is in the tree crowns, whose leaves the bounce leaves to the probes
  under every setting.

  - **Against Falcor in the classroom** (the measure above, × 12,800): clay 0.156 (×0.92), the
    port 0.154 (×0.92); the sun alone 0.174 (×0.91); the sky alone 0.142 (×0.96); textured
    0.149 (×0.98).
  - **The textured Bistro against Falcor** (Lambert, 16 bounces, × 2,871): Exterior_1 0.095
    (×1.01), the probes 0.090; Exterior_2 0.479 (×0.94), the probes 0.439. The gap is the
    materials' (D-083).
  - **The material chart:** the same as with the probes (0.333 against 0.331 at side-30): its
    light is direct.

  **Its cost** (the 5070 Ti, the frame's GPU ms, three runs alternating with the probes):

  | View | frame, the probes | frame, SHaRC | `gi/trace` | update, resolve (overlay) | slots at most |
  |---|---|---|---|---|---|
  | The village's landing | 7.068 | 7.121 | 1.067 → 0.895 | 0.080, 0.142 | 176,615 |
  | The village's room | 8.165 | 8.387 | 1.274 → 1.234 | 0.110, 0.223 | 240,734 |
  | The village's street | 7.131 | 7.553 | 1.082 → 1.385 | 0.040, 0.148 | 119,227 |
  | The Bistro's Exterior_1 | 7.184 | 8.475 | 2.084 → 2.737 | 0.161, 0.382 | 485,769 |
  | The Bistro's Exterior_2 | 6.151 | 6.953 | 1.509 → 1.996 | 0.126, 0.318 | 536,069 |

  - **The overlay's resolve is mostly a wait.** Nsight's GPU Trace (serial, base clocks) times
    the resolve's dispatch at 0.04 ms; its zone also holds the wait for the update to finish.
    The same trace of NVIDIA's RTXGI 2.8.0 sample, SHaRC on its own Bistro at 1600 × 900: update
    0.64–0.70 ms, resolve 0.08–0.09 ms over its 4M slots. Forge's: update 0.48–0.56, resolve
    0.04 over 1M. The passes cost what NVIDIA's do.
  - **The extra rays cost the Bistro most of its millisecond.** A hit nearer than its voxel's
    diagonal goes on along up to two more rays (SHaRC's rule), and the Bistro's far voxels are
    30 cm to 1 m across against the bounce's short rays. The three ways measured (one run each
    for the variants):

    | Short segment | E1 / E2 clay | Landing | Classroom: whole / sky alone | E1 frame over the probes |
    |---|---|---|---|---|
    | Up to two more rays (SHaRC's rule, kept) | 0.048 / 0.101 | 0.089 | 0.110 / 0.097 | +1.29 ms |
    | The probes stand in | 0.038 / 0.077 | 0.090 | 0.094 / 0.196 | +0.36 ms |
    | More rays within 8 m only | 0.038 / 0.085 | 0.094 | 0.109 / 0.166 | about +0.5 ms |

    The probes' version is nearer the path only because their excess (their sky through thin
    walls, D-088 above) offsets SHaRC's deficit: the classroom's sky alone doubles its error.
    SHaRC's rule is kept, as its guide has it, until that deficit is found.
  - **SHaRC reads 3 to 7 % dark** against Forge's path, in steady state:
    - not convergence: frames 880 and 1,780 measure as 280 (0.100, 0.102 against 0.101);
    - not its fixed point: `radianceScale` 1e6 and 1e7 measure as 1e5;
    - partly its resampling: without it (full 8-bounce paths, 8 vertices carried back), the
      classroom 0.110 → 0.076 (×0.94 → ×0.96), Exterior_2 0.101 → 0.094. The rest is open.
    NVIDIA's own SHaRC, against its own path tracer in its sample (its Reinhard undone, 128
    frames, 8 bounces): its Bistro 0.022 (×1.01), its LivingRoom 0.045 (×1.02).
  - **Not deterministic:** two identical runs differ in 25,000 to 30,000 pixels from frames 50 to
    100 (8 levels at most, ꟻLIP's mean 0.001). SHaRC's insertion takes the first free or
    matching slot by an atomic compare-exchange, in the threads' order, and the cache feeds on
    itself. Its documented settings do not change it (`HASH_GRID_LIMIT_EMPTY_SLOTS` 0 tried). The
    probes stay byte-identical, and the A/B harness (D-016) needs that, so SHaRC stays off by
    default: the owner's choice. Vulkan's synchronisation validation: no errors.
  - **Still to check:** the deficit's rest; the Bistro's half-full cache (the design raises the
    size past a third, but the resolve's cost grows with it); the village's thin walls and floors;
    the AMD iGPU (`FORGE_GPU=amd`). These go into D-083's calibration table.

## D-089 — The Bistro metered by day, half a stop brighter than the shared meter ✅ (proposed 2026-10-30; the owner's go the same evening; D-083 step 1)

**Why:** the owner, 2026-10-30: "it's darker than the reference pictures we have in our directory
by a lot".

**Measured** at NVIDIA's two views (Exterior_1's from D-084, 4.5 px; Exterior_2's fitted since,
2.6 px, NVIDIA's 1.1° roll left out), 1920 × 1061 as the pictures, frame 280 of 300 under
`--fixed-step`, with `--capture` and `--capture-linear`, on `main` at b4d3db16. The EV a
picture was drawn at is fitted on 8 × 8 blocks of luma, from a linear capture through Forge's
`aces-narkowicz`. An offline mirror of the meter on the same captures gives the engine's EVs
within 0.05.
- **The Bistro is not metered.** The physics lab's look draws every scene at a fixed exposure
  (`metered: false`, for its compared captures), and the authored scenes (the Bistro, the
  chart, the classroom) took that with the rest. Without `--day` or `--time-of-day` the frame
  is drawn at the shared app's `--ev100` default, 14: a stop over sunny 16, chosen for AgX
  punchy (#220). The two views' PNGs fit 14.00 and 13.98. The "automatic exposure" measured
  earlier that day was this fixed one.
- **NVIDIA's pictures** fit EV 11.74 (Exterior_1) and 9.34 (Exterior_2) on Forge's light: 2.3
  and 4.7 stops brighter than the lab's frames. On Falcor's own renders (`falcor/`, a unit
  2,871 cd/m²) they fit 11.26 and 9.06. Exterior_1 is at Falcor's default exposure (11.22 in
  Forge's units), not one set for that view. Exterior_2 was set 2.2 stops brighter, 0.32
  stop over what Falcor's automatic exposure (0.042 over the frame's log-average) gives there.
  Forge's light is 0.3 to 0.5 stop over Falcor's in both views: its Lambert diffuse against
  Falcor's renormalised Disney (D-086).
- **Metered** (`--time-of-day 0.5`), the shared meter (the log-average of the 50th to 98th
  percentiles, a stop of compensation) draws Exterior_1 at 12.70, a stop under NVIDIA's, and
  Exterior_2 at 9.32, on it. On Falcor's light the mirror puts it 1.03 and 0.13 stop under.
  So the meter already agrees with the picture whose exposure was chosen; Exterior_1's gap is
  to Falcor's default.
- **Not the tone curve.** At the fitted EVs, Narkowicz's curve gives both pictures' luma
  quantiles within a few levels (Exterior_1 in the engine at 11.74, 10th, 50th and 90th
  percentiles 18, 87, 214 against 19, 89, 221; Falcor's Exterior_2 at 9.06: 3, 30, 223 against
  4, 31, 224): one global curve accounts for them. Narkowicz shows the metered key brighter
  than AgX punchy, not darker: 0.171 against 0.115 linear at a stop of compensation (plain AgX
  0.240). To match AgX punchy's grey on screen, Narkowicz would need 0.4 stop less
  compensation, which makes Exterior_1 darker still.
- **Not the light.** Forge's linear light is within the 0.3 to 0.5 stop above, and the
  earlier measurement found the same darkness with the probes alone and with SHaRC.
- **Local exposure** (D-067) lifted the views at EV 14 only because the frames lay 2.3 and
  4.7 stops under the pictures.

**The meter's band against both pictures** (the mirror on Forge's linear captures: the EV each
band asks for at a stop of compensation, less NVIDIA's, positive darker; then the one
compensation that splits the two views' errors):

| Band (percentiles) | Exterior_1 | Exterior_2 | Splitting compensation |
|---|---|---|---|
| 10th–90th | −0.58 | −2.38 | −0.48, ±0.90 |
| 25th–95th (the village's, D-067) | +0.06 | −1.45 | +0.31, ±0.76 |
| 40th–98th | +0.64 | −0.50 | +1.07, ±0.57 |
| **50th–98th (shared)** | **+0.96** | **+0.03** | **+1.50, ±0.47** |
| 60th–98th | +1.30 | +0.63 | +1.97, ±0.34 |
| 80th–95th | +2.06 | +2.10 | +3.08, ±0.02 |

NVIDIA put the two views 2.2 to 2.4 stops apart. A meter averaging the frame puts them 3.3
(the shared band) to 4.2 stops apart (the 10th–90th): Exterior_2's shaded terrace fills most
of its frame. The spread falls steadily as the band rises, so some band was bound to fit the
one gap between two frames: the 80th–95th's near-zero spread is a fit to two pictures, not a
rule. The village shows what it would cost (below): the hall, the door and the room 1.6 to 1.9
stops brighter, a band keyed on the sunlit doorway, the fault D-067 moved the village's band
down for.

**In the engine** (mean luma 0–255; |Δ| the mean absolute difference of 8 × 8 block luma
against the picture; the last row is the floor the scene's own differences and the views' fit
leave):

| Exposure | Exterior_1: mean, \|Δ\| | Exterior_2: mean, \|Δ\| |
|---|---|---|
| NVIDIA's picture | 103 | 65 |
| Today: fixed at EV 14 | 40, 64 | 15, 54 |
| Metered, shared (+1) | 71, 36 | 66, 29 |
| **Metered, +1.5 (proposed)** | **86, 26** | **76, 32** |
| Metered with the village's look | 110, 26 | 111, 56 |
| Fixed at the picture's EV (11.74, 9.34) | 101, 21 | 66, 29 |

**The proposal:**
1. **The Bistro metered by day:** its look's `metered`, with the shared band and adaptation
   speeds. `--fixed-exposure --ev100 E` keeps comparisons with Falcor at one exposure, as now.
   The chart (a measuring card) and the classroom stay at their fixed exposures here.
2. **A compensation of +1.5 stops in the Bistro's look by day,** against the shared +1. With
   two pictures it is a fit, not a calibration: it splits the meter's stop at Exterior_1 (to
   Falcor's default exposure) against half a stop at Exterior_2 (whose chosen exposure the
   meter already gives). In the engine: Exterior_1 at 12.20, half a stop under the picture, and
   Exterior_2 at 8.82, half a stop over. On Falcor's light the same split is +1.58. A night
   start (`bistro::night_start`, as for the lamps) keeps the shared value until the night is
   measured against NVIDIA's Night picture.
3. **What changes in code:** `Look` takes an optional compensation, and
   `--exposure-compensation` becomes optional so that a value given on the command line still
   wins over the look's. Both sit in the shared app (`city-blocks`), but every default stays as
   it is: the meter, the shared compensation, the bands, the tone curves and local exposure.
   The village (AgX punchy, the 25th–95th band, local exposure) and the city keep their images.
   Expected, and run before the change is pushed: the map check matches its accepted set to
   the pixel, and the day check's sentinels, which never draw the Bistro (its files are
   git-ignored), pass unchanged. `docs/demos/physics-lab.md`'s Exterior_1 command takes
   `--fixed-exposure`.

**Cost:** none on the GPU: the histogram already runs in every frame the lab meters (0.02 ms,
D-022).

**Patents:** nothing new. D-022's meter as it is, with one constant per look.

**Considered:**
- **Metering alone, at the shared +1** (step 1 without 2): Exterior_2 exact, Exterior_1 a stop
  dark, the view the owner compares first.
- **The village's look** (the 25th–95th band, local exposure, shadow contrast 0.55), measured in
  the engine: Exterior_1 at about the picture's mean but its shade lifted (10th percentile 37
  against 19), Exterior_2 at 1.7 times the picture's mean, as far off as today's.
- **A compensation matched to the curve's grey** (Narkowicz −0.4 against AgX punchy):
  Exterior_1 1.4 stops dark.
- **A fixed exposure at Exterior_1's 11.74:** right there, 2.4 stops dark at Exterior_2, and no
  adaptation on a walk from the street to the terrace.
- **A band in the highlights** (the table): a fit to two frames, above.
- **Falcor's automatic exposure** on Falcor's light: 1.75 and 0.32 stop under the pictures,
  further from them than the shared meter.
- **A shared change fitted to the Bistro**, measured on the map check's ten views (`city-plan
  --village 600 --time-of-day 0.75`, the mirror on their linear captures; the village meters
  EV 9.7 on the landing to 15.1 on the green). The 80th–95th band at +3.08 would brighten the
  hall, the door and the room by 1.6 to 1.9 stops and the rest by 0.8 to 1.4, and darken the
  main street by 0.3. A shared +1.5 brightens every view by half a stop. The shared band at +1,
  the village's before D-067, darkens them by 0.1 to 0.9.

**Decided** (the owner, 2026-10-30): "for D-089 I will follow recommendations", then "Yes, both
steps". Steps 1 and 2 as proposed.

**Built** (2026-10-30):
- **Code:**
  - `Look::exposure_compensation`.
  - `--exposure-compensation` is now optional (`metered_compensation`: the flag, else the look's
    by day, else the shared stop, `EXPOSURE_COMPENSATION`). A unit test covers the order.
  - The lab's look meters the Bistro with `bistro::EXPOSURE_COMPENSATION`, 1.5.
  - The night is the environment map's rule (`by_night`, `--time-of-day` from 1 to 2), not
    `bistro::night_start` (past 1): at exactly 1 the map is left out and the shared stop kept
    with it.
  - Local exposure's middle grey follows the compensation, as before, so a Bistro run at
    `--fixed-exposure` with local exposure turned on keys it at 1.5.
- **In the engine:** Exterior_1 at 12.20 and Exterior_2 at 8.82 (the PNGs' fits, as the table's
  proposed row). `--time-of-day 0.5` gives the same 12.20; `--exposure-compensation 1` gives
  the shared meter's 12.70; `--fixed-exposure --ev100 11.74` holds its exposure (11.72 by the
  fit).
- **Unchanged, to the pixel (ꟻLIP 0) before and after:** the Bistro's night (`--time-of-day 1.5`)
  at the path's first view and at Exterior_1's, the chart and the classroom.
- **The checks:**
  - The day check (the sentinels on both paths with validation, 1,128 tests): every capture 0 px
    from the accepted set, but #71's flake.
  - The map check: 12 of its 13 views the same to the pixel with D-089 and without; the walked
    way in within ꟻLIP 0.0007, run-to-run noise. Both runs drift 0.002 to 0.004 (mean ꟻLIP) from
    the set accepted at 17:33: NRD's rebuild at its v4.18.0 tag (eee73e3d, 18:13), which moved the
    sentinels alike, not D-089.

**Open:** whether the classroom, an inside lit through its windows, should be metered too. The
chart stays a card at one exposure.

**A scene's own exposure range (2026-11-04).** A look may also give the metered exposure's lowest
and highest EV100 (`Look::exposure_range`, `AutoExposure::range`), so a dark room is shown as much
darker than the street as its authors' pictures show it. The private district (D-083 step 4) takes
its source's limits at the hour, placed on Forge's EV100 by one offset fitted on the source's
sunlit street frames (shown at its ceiling). Against 14 of the source's benchmark frames matched to
the path (their edges against a capture every 0.25 s), the frames' geometric-mean luminance was
+0.76 EV off on average (rms 1.60) under the app's range, and +0.17 (rms 0.54) under the source's;
its bar from 2.45 EV too bright to within half a stop. A range that is not one (inverted, not
finite) is ignored with a warning: `clamp` would panic. No other scene gives one.

## D-090 — Which light under the bounce, by the calibration ✅ (proposed 2026-10-31; D-083, D-088; the owner's go the same day: option 2)

**Why:** the owner, during D-088's SHaRC: "Compare and see what's best and work real-time". The
first baseline (D-083, `tools/gi-compare.sh`, twelve views) measured the probes (the default),
SHaRC (`r.gi.cache sharc`, off by default) and the port (set aside for its patent, D-088) against
Forge's path tracer and Falcor; the night's faults it found are fixed or filed (D-087's
amendment, #368, #369).

**What it says:**
- **Indoors** (the classroom, the village's landing and room), SHaRC is three to five times nearer
  Forge's path than the probes: the landing 0.110 against 0.507, the room 0.052 against 0.147,
  the classroom 0.089 against 0.320. Against Falcor in the classroom, SHaRC 0.149 and the probes
  0.405, where Forge's own path is 0.151.
- **Outdoors by day** (the Bistro, the village's street), the probes are as near or nearer (the
  textured Exterior_2 0.085 against 0.121), and SHaRC costs up to 1.2 ms more (the Bistro's
  Exterior_1 6.89 → 7.87 ms). In the village SHaRC costs −0.05 to +0.3 ms.
- **At night** the lamps' direct light rules: the three alike (0.549 from Falcor in the Bistro);
  the lamps themselves are the next fault (#369). SHaRC costs 2.6 ms more there. **The lamps'
  fault fixed** (#369, #370: a flat piece's lamp its rectangle, D-087), the rooms lit by their
  own lamps rank as by day: SHaRC the nearest or level (the bedroom's clay 0.237 from Falcor,
  the kitchen's 0.092, the Cornell box 0.096), the port level where it holds (the kitchen
  0.097, the box 0.092) and unstable in the bedroom (#371), the probes last (0.849, 0.128,
  0.207; the clay against D-083's corrected, diffuse references). The bedroom's light comes
  from behind curtains after many bounces, which the probes cannot hold.
- **SHaRC is not deterministic:** two runs differ by a few levels (its insertion order, D-088),
  and the captures' rule of 0 pixels between runs cannot hold under it.

**The options:**
1. **SHaRC everywhere by default:** the best indoors, up to 1.2 ms by day and 2.6 ms at night in
   the Bistro; the captures and the A/B harness run with `r.gi.cache off`, and
   `tools/gi-compare.sh` measures SHaRC (by error, not by pixels).
2. **SHaRC indoors, the probes outdoors:** the bounce's hit reads SHaRC where the probes' room
   grids are (D-078: the village's houses), the probes elsewhere. The best of both measured, the
   cost only indoors; one more branch in the bounce (not the resolve, #355). The captures as in 1.
3. **The probes alone, finer indoors:** the room grids at 0.5 m (the classroom's leak, ×1.35,
   went to ×1.00 at 0.5 m, D-088), deterministic; their cost grows with the rooms seen.
4. **NRC** on the same hook (the owner's "ok for the trial"): to measure with the same tool before
   choosing.

**Recommended:** 2, the lamps' fix (#369) now in, with NRC measured beside it before it is made
the default: indoors is where the probes fail and SHaRC wins, and outdoors SHaRC costs without
gaining. To decide by the owner.

**Decided** (the owner, 2026-10-31: "D-90 I will follow recommendations"): option 2, SHaRC under
the bounce where the probes' room grids are, the probes outdoors; NRC measured on the same hook
before either becomes the default everywhere (its download asked for first). The captures and the
A/B harness keep `r.gi.cache off`; `tools/gi-compare.sh` measures the choice by error.

**The design** (2026-10-31, before code):
- **Where "indoors" is:** a bounce's hit inside a probe room grid (D-078: the village's houses'
  rooms, a region other than the outside's), or anywhere in a scene that declares itself indoors
  (the lab's authored rooms: the Cornell box, the classroom, the bedroom, the kitchen, which have
  no room grids). The evidence for SHaRC came from both (the village's landing and room, the
  classroom, the rooms at night).
- **`r.gi.cache auto`**, the new default: the bounce's trace (`diffuse_trace_main`, its SHaRC
  build) asks SHaRC at an indoor hit and the probes elsewhere; `sharc`, `port` and `off` stay as
  they are, for the comparisons.
- **SHaRC's update** walks its paths from the pixels whose own surface is indoors (the same
  test), so a street view does not pay for voxels nothing reads (Exterior_1 by day: 1.2 ms with
  SHaRC everywhere).
- **Determinism:** SHaRC's voxels follow its insertion order (D-088), so the captures, the A/B
  harness, the map check and `tools/gi-compare.sh`'s probes column pin `r.gi.cache off`, and the
  checks of `auto` measure by error.
- **Checks:** `tools/gi-compare.sh`'s indoor views under `auto` as under `sharc`, its outdoor views
  as under `off`; the village's landing and room; the cost by day and at night; the house doors
  (an indoor hit seen from the street, and the street from a room).
- **NRC** on the same hook afterwards: its download (filename, source, size) asked for first.

**Built (2026-10-31): `r.gi.cache auto`.** `sharc_here` (meshlet.slang) answers both for the
bounce's hit and for SHaRC's update (`sharc.slang`): the frame's `FLAG_INDOORS`
(`CullFlags::INDOORS`, a scene's `Look::indoors`: the lab's authored rooms and the models lab's
room), else the probes' room at the point as the lamps find it (`region_open`). Against Forge's
path (`tools/gi-compare.sh`, one run each, mean |log2| and the frame's GPU ms):

| View | off | sharc | auto |
|---|---|---|---|
| the kitchen, clay (declared indoors) | 0.125, 5.42 ms | 0.073, 6.27 ms | 0.073, 5.76 ms |
| the Bistro's Exterior_1, clay | 0.028, 6.64 ms | 0.048, 7.66 ms | 0.028, 6.95 ms |
| the village's room (its room grids) | 0.146, 7.23 ms | 0.052, 8.09 ms | 0.067, 7.80 ms |
| the village's street | 0.116, 6.81 ms | 0.116, 7.13 ms | 0.116, 6.97 ms |

Indoors it is SHaRC's, outdoors the probes'; in the village's room the hits past its door and
windows take the probes. Outdoors it still costs 0.15–0.3 ms over the probes alone: SHaRC's
other passes run though nothing reads them there. The default images: `ast240`,
`city60`, `lab-lamps60`, `lab-materials60` 0 px.

**Skipped where no room is (2026-10-31):** a scene neither authored as a room nor with rooms its
probes know (`Probes::has_regions`) creates and runs no SHaRC under `auto` (`sharc_wanted` in
city-blocks), since `sharc_here` answers no at every pixel there. The Bistro's Exterior_1, the
frame's GPU ms, two runs each alternating: `off` 6.947, 6.842; `auto` 6.865, 6.896 (it was
0.3 ms over). The kitchen (declared indoors) and the village's room (its rooms) still run
`gi/sharc update` (0.099 and 0.093 ms). A street of the village still pays its 0.16 ms: its
houses' rooms are in the probes, and a hit through a door may be indoors.

**The review's fixes (2026-10-31):** a doorway (a front door, a shopfront: a volume between the
outside and a room) answers by the side its point is on, by the probes' ramp across it (0 at its
stored `region`'s face, 1 at its `other`'s), where its room side had taken the probes too (the village's room and street
under `auto`: 9 and 193 px of 1.44 M differ, their error to the path unchanged, 0.067 and 0.116);
`--no-cfg` is a scripted run as `FORGE_CFG=none` is (the kitchen keeps the probes, 5.49 ms).

**The default since 2026-10-31** (the owner: "Follow recommendations"): `auto`, but in a scripted
run (no settings file, `FORGE_CFG=none`, which every script sets, or `--no-cfg`:
`vars::gi_cache_default`),
which keeps the probes for images equal from run to run (`docs/PROCESS.md`). The captures
unchanged (`ast240`, `city60`, `gallery60`, `lab-lamps60`, `lab-materials60`: 0 px); the kitchen
run as the owner runs it takes SHaRC (`gi/sharc update` 0.10 ms, the frame 5.54 → 6.18 ms).

**NRC measured on the same hook (2026-10-31, the owner: "So go NRC").** NVIDIA's NRC 0.16 behind
`r.gi.cache nrc` (`crates/forge-render/src/nrc.rs`, `shaders/nrc.slang`,
`crates/forge-gpu/src/nrc.rs`; `docs/research/sharc-and-nrc.md` has the integration's findings):
- **The hook:** the bounce's hit makes NRC's query and adds its own sun, glow and lamp exactly,
  as under SHaRC. Training paths start from the visibility buffer's surfaces at NRC's training
  size (197 × 111 at 1080p) with their first vertex at their first hit. A custom resolve adds
  the answers to the bounce's light before REBLUR.
- **The choices, measured** (classroom, Exterior_1 and Exterior_2 in clay, mean |log2| against
  Forge's path):
  - The hits' own light added exactly (0.066, 0.028, 0.102) beat NRC's default of holding it in
    the cache (0.114, 0.085, 0.158): the cache blurs the sunlit patches at the bounce's hits.
  - Training every first vertex rather than NRC's 5 % changed nothing measurable, so NRC's 5 %
    stays.
  - NRC's light scale follows the frame's exposure, 4 stops over the light the frame averages
    (`nrc::expected_radiance`). Fixed at 1, a sunlit white surface's light, NRC lost the lamp-lit
    rooms at night (about 13 stops over their average): bedroom in clay 0.693 → 0.231, Cornell
    box 0.317 → 0.051. At 0 stops it lost the day instead (classroom 0.243).
- **Error and cost** (`tools/gi-compare.sh`, 1600 × 900; error is mean |log2| of 16 × 9 tiles
  against Forge's path, the frame's light over the path's; cost is the frame's GPU ms, the mean of
  two 1,200-frame runs):

  | View | probes | SHaRC | NRC | frame: probes, SHaRC, NRC |
  |---|---|---|---|---|
  | Bistro E1, clay | 0.027 | 0.057 | 0.028 | 6.77, 7.91, 9.16 |
  | Bistro E2, clay | 0.132 | 0.104 | 0.097 | 5.73, 6.32, 8.16 |
  | Bistro E1 | 0.052 | 0.065 | 0.037 | 6.76, 7.80, 9.24 |
  | Bistro E2 | 0.091 | 0.126 | 0.122 | 6.27, 6.77, 8.35 |
  | Bistro, night | 0.097 | 0.097 | 0.099 | 21.41, 26.88, 24.54 |
  | Classroom, clay | 0.360 | 0.117 | 0.077 | 3.89, 4.23, 5.54 |
  | Classroom | 0.306 | 0.100 | 0.067 | 4.14, 4.43, 5.88 |
  | Chart | 0.033 | 0.030 | 0.018 | 3.32, 3.40, 4.37 |
  | Cornell box | 0.226 | 0.113 | 0.051 | 2.99, 3.10, 4.25 |
  | Bedroom, clay | 0.789 | 0.189 | 0.218 | 5.04, 5.57, 6.62 |
  | Bedroom | 0.988 | 0.281 | 0.311 | 5.45, 5.98, 6.96 |
  | Kitchen, clay | 0.136 | 0.083 | 0.092 | 5.19, 5.78, 6.95 |
  | Kitchen | 0.266 | 0.132 | 0.138 | 5.59, 6.29, 7.29 |
  | Village landing | 0.487 | 0.122 | 0.380 (×0.83) | 6.19, 6.20, 8.13 |
  | Village room | 0.144 | 0.058 | 0.130 (×1.08) | 6.97, 7.22, 9.08 |
  | Village street | 0.112 | 0.112 | 0.123 | 6.73, 7.11, 7.98 |

  NRC is the nearest to Forge's path on 7 views, within 0.03 of SHaRC on the lamp-lit rooms,
  and well behind it in the village's interiors. It costs 1.1 to 2.5 ms over the probes,
  nearly all of it NRC's own inference and training (`gi/nrc train`, 1.1 to 2.3 ms; Forge's
  passes take 0.06 ms). SHaRC costs 0.1 to 1.1 ms by day. At the Bistro's night SHaRC's extra
  segments make `gi/trace` 9.0 ms, and NRC is 2.3 ms cheaper than SHaRC.
- **Shimmer** (a still camera, frames 10 apart, pixels that change by more than 2 codes, DLAA):
  - kitchen: probes 0.34 %, SHaRC 0.31 %, NRC 0.31 %;
  - bedroom: 4.6 %, 3.7 %, 3.8 %;
  - Cornell box: 0.002 % each.

  NRC adds none once trained. In its first 100 frames, behind the Cornell box's tall block (AA
  off), 13.8 % against the probes' 11.3 %, then 9.0 %. The bedroom's flicker is its ceiling
  wedges under every technique, the lamps' direct light (#383).
- **The village's interiors take seconds to settle:**
  - **The drift:** still camera, DLAA, the frame's mean light against frame 200.
    - NRC rises +0.35 EV in the landing and +0.16 EV in the room until about frame 800 (13 s at
      60 fps), then holds.
    - The probes move by −0.06 and 0.00 EV, SHaRC by +0.04 and +0.03.
    - So for seconds after a load or a cut, the interiors' light visibly climbs.
  - **Settled, at frame 1,180:**
    - The landing is 0.079 from Forge's path, better than SHaRC's 0.122.
    - The room settles 14 % bright (0.247, ×1.141); SHaRC is 0.058.
    - Eight training batches a frame (`FORGE_NRC_ITERATIONS`, 1.2 ms more) reach the landing's
      settled light by frame 280, but the room as bright.
    - Smaller bounds round the camera (±128 m, ±32 m) did not take the room's excess away (0.234,
      0.188). Its cause is open.
- **Not for AMD:** NRC is NVIDIA's (CUDA through `VK_NVX_binary_import`), so AMD keeps SHaRC
  or the probes.
- **Where it stands:** NRC beats SHaRC by day, in the classroom and the Cornell box. It costs
  about 1.5 ms more, it ties SHaRC in the lamp-lit rooms, and it trails in the village's
  interiors (slow to settle, the room bright).

  **Recommended:** keep `auto` (SHaRC indoors) as the default and NRC behind `r.gi.cache nrc` on
  NVIDIA. Before NRC is offered as the cache on NVIDIA's cards:
  1. find the room's excess;
  2. settle faster after a cut (more batches for the first seconds, then NRC's 4);
  3. measure it in a walk, where it trains as it goes.

  To decide by the owner.

## D-091 — A tree's leaves as shoots at its twigs' tips, a prototype behind `--twig-leaves` ✅ (proposed and decided 2026-10-31, #379; the owner's go on the look the same day: "Yes, finish it"; D-063, D-075)

**Why:** the owner, 2026-10-31: "tree foliage or crown are often very dense same for bushes and
garden hedges [...] They often intersect and are currently kind of flat planes. Why are they so
dense?", then "a few leaves at the end of some twigs themselves attached to the trees are enough",
and on the first prototype: "this tree is too uniform not looking natural, we can distinguish some
kind of pattern", "the leaves are evenly spaced". The grown plants (D-063) are sound; the mesher
multiplies their leaves. Cards round overlapping lumps (16 to a tree's lump, sprays up to 1.2 m)
and opaque cores give 2.6 to 6 times the published leaf area over a crown's projection (7 to 17,
where an olive measures about 3.5, from 1 to 7, an open-grown holm oak 1.0 to 1.6, kermes garrigue
2 to 3.3): a ray through a crown crossed 6 to 46 card planes, 2.5 to 16 of them opaque. And every
card was the same picture, a pinnate spray with its leaves at even steps, stamped thousands of
times over a crown, most facing the same way.

**What it is** (city-plan's `--twig-leaves`; the default stays on the lumps):
- **Wood and twigs** (`forge_geom::flora::shoot_mesh`): a tree's wood down to 2 cm as before
  (7-sided tubes, `SHOOT_WOOD`), every finer twig as a 5-sided tube that starts at its own radius
  inside the wood (from its parent's, a twig stood out of a limb as a horn); a shrub's twigs of
  2.5 mm and more.
- **Shoots at the twig tips** (`shoots`): as many cards as the species' leaf area asks over its
  crown's footprint (`Look::leaf_area`, from the measures above, set for the look per species in
  `mediterranean.toml`), their leaves the species' length (`Look::leaf`). Shared unevenly: each
  branch off a limb its own leafiness (0.4 to 1.6), ten at a tip at most and the rest back along
  its twig (forty in all: capped at the tip, a young holm oak had 45 % of its leaf area). At a
  twig's end the first carries on its way and the others fan out at about the branch angle,
  turned about the divergence; at a node a twig goes on from, all fan out. Each faces out of the
  crown and up, rolled up to 46° its own way about its stem, folded 9° to 35°, sized 0.75 to 1.25,
  mirrored half the time, keyed by the plant's seed.
- **The shoots' atlas** (`forge_render::textures::shoot_leaves`, 2048 texels, a cell of 512 as a
  spray's quarter): four branchlets for each spray, each its own. A main shoot leaning and bending,
  side shoots at uneven places, some branching again, the leaves at uneven steps that crowd
  towards the tips, a node here and there bare, set as the species carries them (one to a node
  turned by the golden angle for the oaks and planes, pairs of needles for the pines, opposite
  pairs a quarter turn apart for the olives), a leaf turned towards or away from the viewer drawn
  shorter, in front of its stem or behind it. Each picture as large as its cell holds it; its
  leaf's share of the card and its cover are kept in `LeafSpray::shoots`, which a test holds to
  the painted atlas.
- **What stays the lumps':** the probes' proxies, the shells and the far shadows' proxies
  (D-075's ladder), so the far steps do not match the shoots yet.

**Numbers:**
- **Leaf area:** every plant carries 0.83 to 1.16 of what its species asks (each card's area times
  its picture's cover; the few-dozen-card shrubs stray the most), against 2.6 to 6 times with the
  lumps. Opaque layers per ray 0.7 to 4.4 (2.5 to 16 with the lumps), planes 2.8 to 19 (6 to 46).
- **Triangles:** a grown holm oak 228 000 (wood 54 000), a lime up to 210 000, an olive 60 000,
  where the lumps take 8 000 to 33 000: the test's budget for a tree is 250 000 for now.
- **The village's GPU** (the map check's views, DLAA, the 5070 Ti): 4.9–8.2 ms with the lumps,
  6.4–9.9 ms with the shoots. The probes' rays +0.8 to 1.4 ms, the geometry +0.6 to 1.2, the
  bounce's trace +0.45 to 0.6: the twigs and the cards in the traced scene, which the probes and
  the bounce meet only through their shadow rays and the cards' alpha.
- **Off, nothing moves:** the village's thirteen views without the flag 0 px from the same tree's
  run before the change (the atlas's painter now paints a grid of cells; the leaf atlas's bytes are
  the same).

**Patents:** the shoots are each plant's whole foliage, not a step on an LOD ladder that drops
leaves and enlarges the rest (Pixar's US 7,973,789 B2, D-075). The growth (Pałubicki et al.
2009) is D-063's. No other patent met.

**Next** (#379), before it is the default:
- The crown's ladder from the shoots: shells and impostors that look as the shoots do, and the far
  mips (a cell of a texel or two keeps its cover rounded up, so cards a pixel across could read
  solid).
- The cost: the probes' and the bounce's rays through the twigs and cards, fewer card triangles,
  measured per pass; the triangles to a few times the lumps'.
- The owner's walk among the village's trees.

## D-092 — A settings screen as games have: live where the engine can, kept for the next start where it cannot ✅ (the owner's ask, 2026-10-31, #377)

**The owner (2026-10-31):** "a game menu that looks more professional, with graphics settings:
DLAA, DLSS upscaling, FSR upscaling and other parameters, like games often do; some display
settings if needed. It would be great when it's applicable without restarting the engine; if
not possible, do as game engines often do: 'it will be applied after you restart' or similar.
But be careful that your tests always use the test parameters and not what the user persisted
in their changes." Then, on a mock-up: "a glass gaussian transparency effect but almost opaque
still to not get in the way of visibility and take the screen from top to bottom", "or full
screen".

**Built (2026-10-31):**
- **The screen** (`forge_app::menu`, its rows in `menu::items`): Settings opens three pages,
  Graphics, Display, and Sound and controls, in a column the window's full height. Each row
  shows its value between arrows; the lines under the list say what the row does and how it
  applies. A value the machine or the demo lacks is greyed with why (DLAA without DLSS: "DLSS
  needs an NVIDIA RTX GPU and NVIDIA's Streamline, loaded at start"; HDR on a display Windows
  shows in SDR); a setting the demo does not
  register has no row.
  - **Applying:** changes wait in a draft until Apply; Defaults puts the page back to its
    defaults; Back with changes asks first: Apply, Discard or Cancel.
  - **Input:** the keys by their place (D-069; AZERTY's Z, Q, S, D move and change, A and E turn
    the pages), Tab, Enter, Space and Esc; the mouse hovers, clicks (a step forward) and
    right-clicks (a step back).
  - **The size:** the text's height is a 46th of the window's (13 to 64 px), so 1080p and 4K
    read alike, and smaller when a page's rows need it.
- **The glass** (`forge_app::glass`, `shaders/menu_glass.slang`): the frame copied, brought to a
  quarter of its size by 4 × 4 means (no shimmer from a moving picture), blurred by a Gaussian
  of σ 4 there (16 of the window's pixels) across and down, and drawn back over the whole
  window, 80 % a dark tint mixed as the eye sees it (under a 2.2 gamma; PQ's codes as they
  are); the game's HUD and its lens are hidden under it. Five passes (`app/menu glass *`), only
  while the menu shows; a copy rather than a blit and quarter-size images in RGBA16F, so no
  window format needs blits or filtering. Where it cannot be made, the old dimming.
- **Graphics:** the preset (D-081), anti-aliasing and upscaling (off, DLAA where DLSS runs,
  DLSS's and FSR's modes where the demo upscales, FSR's native AA, the render size in the
  description), FSR's sharpening, the sun's shadows, the global illumination (probes, traced,
  traced with SHaRC indoors: D-088, D-090), the ambient occlusion, the ray-traced reflections,
  the local exposure, the bloom and the level of detail. Left out: frame generation (Forge has
  none), a texture quality (no texture budget yet; a mip bias alone would shimmer).
- **Display:** the window mode, its size when windowed (`app.window.width`, `.height`, up to the
  monitor's), the monitor (`app.monitor`: automatic is the first that is not the main one, as
  `FORGE_MONITOR=secondary`, which still wins), VSync (`app.vsync`: FIFO, or mailbox off), a
  frame-rate limit (`app.fps_max`, interactive runs only), HDR (`app.hdr.mode`) and its
  calibration, and a brightness (`r.brightness`: ±2 stops added to the metered exposure's
  compensation, in the galleries, the asteroids and the planet). No exclusive fullscreen:
  borderless presents through the flip model as directly, and a mode change would disturb the
  other monitor.
- **Live, or after a restart:** every row applies at once today; the shell follows the display's
  settings each frame (`follow_display`: the window resized or moved, the swapchain made again
  for a new present mode, the HDR mode switched). A setting read only at start (flagged
  `restart`) is saved for the next one (`Settings::set_user_next_run`) and its row says
  "Applies after you restart"; none of the screen's needs it yet, the path is tested on
  `app.gpu`.
- **The user's file** (amends D-081): the user's choices are kept apart from the values in
  force. A choice stays in the file while `forge.cfg` or `--set` overrides it, while a demo
  does not register it, and when it equals the default (the tone curve's default differs from
  demo to demo); only Defaults removes it (`Settings::forget_user`), and leaves a value
  `forge.cfg` or `--set` gave. Before, the next write dropped a choice `forge.cfg` overrode. The
  first launch's choices rank under `forge.cfg` and `--set` as the file's do
  (`Settings::set_user_beneath`), and `--vsync` as the command line does. A value in force that
  is not the user's yet (Alt+Enter's, a window resized by hand, `--set`'s), chosen as it is,
  stays in the draft for Apply to make it theirs.
- **The tests' parameters** (the owner's warning): a scripted run (a frame limit or a capture,
  `AppConfig::scripted`) never reads or writes `user.cfg`, whatever `FORGE_CFG` says;
  `FORGE_CFG=none` and `--no-cfg` keep it out of any run. A scripted run also leaves
  `app.fps_max` aside and presents uncapped as before.
- **Captures are opaque:** the window presents opaque, but a translucent HUD quad left its
  alpha in the captured PNG (the menu's column came out see-through, as #352's bar did); the
  capture now writes alpha 255, as the screen shows it.

**The looks (the same day):** the owner asked what the menu would look like in the Carbon HUD of
their streaming overlays ("with the theme of the project in the directory obs-streaming-project?
(take background and fonts maybe?)"), without its corner brackets and the blue bar down the
column's side, then "keep the one you did before somewhere, it fits more the base theme, but keep
carbon as the current theme for now". `app.menu_theme` chooses:
- **carbon**, the default (`menu::carbon`): the overlays' dark (#0A0B0E), ion blue (#4FD8FF)
  and amber (#FFB454); Orbitron ExtraBold for the titles, Rajdhani Medium and SemiBold for the
  labels, tabs and buttons, JetBrains Mono for the values, the sections and the hints (all OFL,
  `assets/fonts`, `CREDITS.md`); the titles, tabs, values and buttons in capitals, dotted
  leaders from a label to its value,
  a change waiting for Apply in amber and Apply glowing while there is one. The glass is tinted
  as the overlays' page: a violet glow at the top right, an ion blue one at the bottom left and
  a vignette (their faint grid tried and left out, the owner), composed as seen in every
  encoding (a PQ frame decoded first) and dithered so that the glows' long ramps do not
  band in 8 bits. The page's colours are its sRGB codes, decoded as such. Its translucent ones
  are converted for the HUD's blending in linear light (`carbon::Blend`: a light colour at the
  alpha that looks the same, a dark veil at the one that darkens as much); in HDR10 the HUD
  blends on PQ's codes, perceptual as the page's, and takes the page's alpha as it is. The focus
  band is even: a fade across the row broke into steps in 8 bits, which the HUD cannot dither.
- **base** (`menu::base`): the first look, one monospaced face, grey text and an orange accent,
  the column's edges drawn, the glass's plain tint.
- Both share the menu's state, keys, rows and descriptions; each lays out and draws its own
  column. The glyphs: each face rasterised at 26 and 56 px (the titles at 96 too) into atlases of
  16 glyphs a row (one row of the wider faces' cells would pass the 4096 px every GPU takes), set
  from their baseline by each glyph's own advance.

**Checks:**
- **Unit tests** (forge-core, forge-app): the rows each machine and demo gets, the draft and
  Apply, Defaults, the dialog on leaving, the preset turning custom, the tabs, the keys and the
  mouse, the layout from 900p to 4K in both looks, the theme's choice, the user layer (kept choices, `forget_user`), the restart
  path, and the scripted rule (a poisoned `user.cfg` changes nothing in a scripted run, five
  settings in an interactive one).
- **The guard on images:** city-blocks' frame 60 with a `user.cfg` setting `r.aa off`,
  `r.preset low`, `r.ao 0`, `r.brightness 2`, `app.hdr.mode scrgb`, `app.window.width 800` and
  fullscreen, in the settings folder the run reads and `FORGE_CFG` unset: 4 px of 1.44 M differ
  from a clean run (max 5 of 255), as two clean runs differ (1 px, max 3), and the image keeps its
  1600 × 900. The same values on the command line change 32.8 % of the pixels.
- **The screen** captured over the village at 1080p and 4K (`docs/demos/images/game-menu.png`).
  The glass costs 0.05 ms at 1080p and 0.13 ms at 4K in the base look, 0.06 and 0.17 ms in
  Carbon's (`docs/PROFILE.md`).
- **A read-only review** before the push found the first launch overriding `forge.cfg` and
  `--set`, a click on a section's title acting on the focused row, values in force the menu
  could not save, the HDR row wrong at start and `--vsync` making VSync's row inert: fixed,
  each but the last two with a test.

**The galleries upscale (the same day, the owner: "do it here but careful to not change the
configurations of other demos"):** city-blocks, the island, the physics lab and the city plan
(their shared `Gallery`) take `r.dlss.mode` and `r.fsr.mode` as the asteroids do, so the menu's
anti-aliasing row offers DLSS's and FSR's modes there too.
- **The scene at the upscaler's size** (`Gallery::fit_upscaler`): the renderer, the jitter, the
  shadows' and the light's denoisers, the reflections' history and the tank water's scene FSR
  are sized to the upscaler's input. The bloom, the local exposure and the display pass stay at
  the window's size, after the upscaler.
  - The jitter's sequence lengthens with the ratio (`taa::jitter_phases`).
  - The textures' bias adds log2 of the ratio to `r.texture.lod_bias` (D-070).
  - The geometry's error (`r.lod.error`) counts the window's pixels, scaled by the render's
    height over the window's as in the asteroids, so the clusters stay as detailed as on screen.
    The trees' ladder (9 px) and impostors (40 px) count the drawn pixels: in a smaller mode they
    cross nearer by the ratio, since what the ladder guards (cards a pixel or two wide going
    see-through) happens in the drawn image.
  - A mode changed at the console waits for the device to be idle, then resizes what changed. A
    switch that fails goes back to the window's size, every image with it.
- **The defaults are unchanged:** `r.aa` stays DLAA where DLSS runs, else FSR at the window's
  size. The modes apply only under `r.aa dlss`, or `r.aa fsr` with a mode other than native. A
  DLSS mode asked for where DLSS cannot run gives FSR's, as in the asteroids.
- **The presets now reach the galleries:** they set `r.dlss.mode` and `r.fsr.mode` (D-081).
  Where DLSS does not run, `r.aa` is fsr, so a preset below ultra draws the galleries smaller
  (high: FSR's quality, 1.5 times smaller), as in the asteroids. Under DLAA the presets keep the
  window's size.
- **Verified:**
  - The sentinels of the four galleries (`city60`, `city60-noocc`, `city60-fsr`, `gallery60`,
    `shot-island`, `lab-materials60`) differ from the accepted set in 0 pixels, and the village
    with the menu open in 0.
  - Every mode switched live in the village at 1920 × 1080: exit 0, no validation errors. The
    menu's row reads "DLSS quality".
  - The GPU frame, 149 frames: DLAA 6.44 ms; DLSS quality 4.69, balanced 4.34, performance
    4.03, ultra performance 3.55; FSR native 6.43, quality 4.48, performance 3.74
    (`docs/PROFILE.md`).
  - A read-only review found the geometry's error counted in drawn pixels, Streamline asked at
    each resize under DLAA (which never needs it), and a failed switch leaving the images at
    different sizes: fixed.

**Open:** DLSS's ultra performance at a 1080p window draws the near leaf-card trees dark and
speckled at 640 × 360, with the ladder off too (#382; quality to performance look right); key
rebinding in the menu (`bind` stays the console's); a resolution scale apart from the
upscalers.

*(issue #377; D-069, D-081; docs/demos/console.md, "The game's menu")*

## D-093 — By day, a wood's shade kept darker than the streets: the exposure a quarter of the way ✅ (the owner, 2026-10-31: "0.25"; D-022, D-046, D-067)

**Why:** the owner, walking into the village's wood: "going in the canopy/crown of the trees make
the outside overexposed. Even walking in the forest make the forest light brighter than even
outside of it." At a fixed exposure the wood's floor is 0.3 to 0.8 stops darker than the field at
its edge, as it should be; the automatic exposure (D-022) brought it all the way to middle grey,
1.1 to 1.5 stops brighter than at the edge, so the wood was shown brighter than the field and the
sky between its leaves went white. The village had the full adaptation since 2026-10-16, when at
the street's fixed exposure the ground under the holm oaks was black.

**What games do:** the eye's full adaptation is not what they show. They clamp the exposure per
area, adapt partly to a dark scene (Unreal's exposure compensation curve; Krawczyk et al. 2005
lower the key of a dark scene), and keep rooms adapted (exposure volumes).

**What it is** (`AutoExposure::shade_adapt`, `r.exposure.shade_adapt`):
- **By day, outdoors,** the meter's key under an open daylight's is followed only `shade_adapt` of
  the way: the scene is shown `(1 − shade_adapt)` times the stops it falls under it darker than
  adapted fully. The night keeps its own darkening (D-046), the shade fading out with it.
- **The open daylight** is a sunlit street's key: level ground's light from the sun and the sky at
  the hour (the sky's irradiance read at frame 30) times 0.04 (`DAYLIGHT_KEY`), measured on the
  village's views at noon (its green 0.15, the view over the wood 0.06, the main street with its
  houses' shade 0.04, inside the wood 0.015 to 0.02). A street keeps its exposure.
- **Indoors** (the camera inside one of the scene's rooms, `region_at` over the probes' region
  volumes, D-068) the exposure adapts fully.
- **Local exposure** (D-067) keys on the darkened middle grey, as for the night, or it would lift
  the wood back.
- **The village's look** takes 0.25 (`VILLAGE_SHADE_ADAPT`); 1, every other scene's default,
  changes nothing.

**Numbers** (the wood at noon, the walk in from the field and the view up under the
canopy; `reports/2026-10-31-forest-light/5-shade-strengths-1-05-025-0.png`): the wood shown 0.8
(0.5), 1.2 (0.25) or 1.5 (0) stops darker than adapted fully, 0 at the field's edge, a room's EV
10.83 and the main street's 11.78 at every strength. The map check: its twelve views within the
runs' noise (mean ꟻLIP 0.0005 to 0.003 against the same tree's run before), the walk into a house
(`enter`) 0.046: entering through its shaded porch the hall starts darker and catches up within a
second or two (the same frames 0.002 apart from frame 600); accepted.

**Left as it is:** the daylight is read once a run, at frame 30 (a clock moving the sun far would
leave it stale until the next run).

## D-094 — The game's frame: 120 fps at 2560 × 1440 through DLSS or FSR, never under 30 ✅ (the owner, 2026-11-01; D-070, D-082, D-083)

**The owner's words:** "The engine will have to have good performance, strongly desired is at
least 120 fps in dlss or frs at my screen resolution (WQHD 2560x1440) for a very good rendering
quality. My guess is that optimizing DLAA would always be useful for dlss or frs later. We have
to stay fluid even when moving fast, and in complex scene moving around the game world. Going
below 120 fps can happen but always above 30 ftp (60 fps minimum prefered) and would be for
cinematics with very detailed scenes for example."

**What it sets:**
- **The target:** 120 fps, 8.33 ms a frame for the GPU and for the CPU's frame thread alike, at
  2560 × 1440 (the owner's screen) through DLSS (NVIDIA's cards) or FSR (elsewhere, D-082), at a
  very good image, on the RTX 5070 Ti.
- **DLAA optimised too:** the native-resolution mode costs what the upscaled modes cost on more
  pixels, so a pass made cheaper under DLAA is cheaper in every mode; DLAA stays the reference
  image (D-070).
- **The floor:** under 120 fps only where the image asks for it, a cinematic's very detailed
  shot: 60 fps preferred (16.7 ms), never under 30 (33.3 ms).
- **Fluid in motion:** the frame holds while the camera moves fast through complex scenes and
  streams the world in. A still view's mean is not the measure: a fixed path at speed is, with
  its mean, its 99th percentile and its worst frame, and its hitches (a frame over twice the
  budget) counted.
- **Techniques mixed to hold it:** "we can mix techniques for rendering, even using
  rasterization if needed, it quite good in some cases after all" (the owner, the same night).
  Each effect takes what holds the frame at a very good image: rasterized where that is good
  enough, traced where tracing pays.

**How it is measured,** as a game in production: a frame budget per system (the GPU ms of each
pass group, the CPU ms of each thread's work) at 2560 × 1440, under DLSS Quality and DLAA, over a
fixed path flown or driven at speed; the numbers in `docs/PROFILE.md`. The private district (D-083,
step 4) is the first scene held to it, then the demos.

**Amended 🟡 (2026-11-07, proposed): the budget by group, from the private district's baseline.** The owner moved the
effort to it ("the frame budget against your D-094 target"). The baseline is the district on its benchmark path at
2560 x 1440, two runs, frozen with its scores and stills for later work to be checked against. GPU ms a frame by the
profiler's groups (zones on the async queue overlap the graphics queue):

| Group | DLSS Quality now | DLAA now | Proposed budget (DLSS Quality) |
|---|---|---|---|
| Lamps: ReSTIR DI and its denoiser | 4.6 | 9.7 | 1.5 |
| GI: the traced bounce and its denoiser | 3.4 | 7.9 | 1.5 |
| GI's probes (async, overlapped) | 3.0 | 3.6 | 1.5 |
| Shading: the resolve, layered materials | 3.1 | 6.3 | 1.8 |
| Geometry: culls and raster | 1.2 | 1.4 | 1.0 |
| DLSS | 0.8 | 0.9 | 0.8 |
| Fog, sun shadow, the movers' structure, sky, particles, AO, post | 2.2 | | 1.5 |
| **The frame** | **17.1** | **31.7** | **8.1, under 8.33** |

The levers, in the order they pay:
- **The lamps, −3 ms:** fewer ReSTIR candidates, half-resolution reservoirs, and the lists for near lamps.
- **The GI, −2 ms:** the trace at a quarter of the pixels, and the probes' rays capped per frame.
- **Shading, −1.3 ms:** a material level of detail (far pixels read a layered material's flat colour), and the layers cut more tightly by their presence maps.
- **Not the movers' structure:** its zone, about 0.45 ms, counts the wait on the work before it. Refitted, it shows 0.03 ms
  and the frame stays the same (PROFILE, the district's turning meshes).

The CPU's frame thread does about 1.4 ms of work, well within budget. Each cut is checked against the baseline's scores and stills before it lands.

**Amended: the shipped game's benchmark is the floor, with every system running** (the owner,
2026-11-01 morning: "keep in mind that we need, in general, other game systems to run (like
entities managements, network, physics and so on) so the real goal is above 120 fps"; "we will
adapt our minimum fps goals, with all the systems active, to what the benchmark results are
giving. 120 fps on dlss balanced would be nice"; "We have to do at least as good as the benchmark
different modes and settings"):
- **The floor, mode for mode and setting for setting:** Forge with its game systems active
  (entities, physics, network, sound, scripts) runs at least as fast as the shipped city game's
  built-in benchmark on the same machine, in each of its modes and at each upscaling setting. Its
  runs (2026-11-01, RTX 5070 Ti, Ryzen 7 9800X3D, 2560 × 1440, frame generation off; mean / minimum
  fps, then frame times p50 / p99):

  | Mode | DLSS Balanced | DLSS Quality | DLAA |
  |---|---|---|---|
  | Rasterised (no ray tracing) | 169.3 / 125.5 fps; 5.69 / 9.08 ms | 152.3 / 121.2 fps; 6.48 / 9.44 ms | 84.8 / 74.0 fps; 12.12 / 14.10 ms |
  | Ray traced (its "Ultra") | 107.7 / 93.5 fps; 9.16 / 11.84 ms | 95.0 / 84.0 fps; 10.50 / 12.39 ms | |
  | Path traced, ray reconstruction | 76.4 / 67.1 fps; 12.87 / 15.51 ms | 65.0 / 56.7 fps; 15.14 / 18.50 ms | 35.8 / 29.7 fps; 27.35 / 36.03 ms |

- **Forge's modes to match them:** a rasterised mode (no traced light: its lighting precomputed,
  as the game's own baked data does), the traced mode Forge draws now (traced shadows and bounce
  light, between the game's ray-traced and path-traced looks), and its path-traced reference; each
  held to its row, as presets the settings screen offers (#377).
- **The wish:** 120 fps under DLSS Balanced with every system running.
- **The game's systems are measured in the same frame** as they come in (D-062's mission maps,
  #266): their CPU ms per thread and the frame thread's wait on them, in `docs/PROFILE.md`.

## D-095 — Layered materials drawn in the resolve: tiled layers blended by masks, four a texel ✅ (proposed 2026-11-01, #384, and followed under the owner's word of the night, "Follow recommendations until 10 am"; D-094, D-083 step 4, D-074)

**Why:** an authored city's surfaces come as stacks of tiled layers (a base and up to twenty more,
each a tiling material template tinted from a palette, its roughness and metalness remapped, blended
by its own mask with a microblend at the mask's edge). The private district of D-083's step 4 is made
of them: about 90 % of its 73,000 mesh chunks' materials. Forge's materials are one row a section;
flat colours folded from the layers lose the tiling detail and the edges the look lives in. The
research (`research/layered-materials.md`) compared three families: the layers per pixel in the
resolve, a runtime composited virtual-texture cache, and offline bakes.

**Proposed (the research's recommendation):**
1. **Layers evaluated in the resolve** (the visibility buffer's material pass), one function for a
   layer: its template's maps sampled at its tile, its palette colour, its levels, its microblend
   sharpening the mask's edge, folded front to back.
2. **The masks compacted when the content is cooked:** each mask texel keeps its four strongest
   (layer, weight) pairs, so a pixel samples at most four layers, never twenty masks to find the
   zeros (Call of Duty's layer index); an early-out under a small weight.
3. **Uniform loops:** per 8 × 8 tile, the loop runs over the union of its pixels' layers (no
   assumption on the wave's width, CLAUDE.md's cross-vendor rule).
4. **The reference image:** the same function over every layer and every mask, uncapped
   (a switch), to measure the cap against.
5. **Measured** on fixed paths through the district at 1707 × 960 (DLSS Quality at 2560 × 1440): the
   resolve's time within about 1–1.5 ms, a heat map of active layers, ꟻLIP of the cap against the
   reference at a fixed exposure.
6. **Not now:** a composited mesh-UV cache (family b) only if (a) misses the budget, and only after
   the owner has seen AMD's US9728002 and US12444121 (shading only uncached texels; gradient
   chaining in decoupled shading); bakes (c) for the far levels of detail.

**Patents:** none found on blending layered materials per pixel; the cache's family meets the two
AMD patents above (the table in the research), which stays the owner's call before any of it.

**Amended before the build: the masks kept whole, the loop over each tile's present layers**
(2026-11-01, the owner: "we certainly are missing textures and other elements [...] I think it's
the layer materials"). Measured on the benchmark's district (1,236 layered materials): the median
shows seven layers over 1 % of its surface and 62 % show five or more, so four layers a material
drops most of them; and four (layer, weight) pairs a texel cannot be filtered (a bilinear sample
mixes texels whose four differ: seams). So, in place of item 2:
- **The masks stay each layer's own,** an array of block-compressed single-channel images per
  material (BC4, mipped), sampled filtered: the front-to-back blend (each layer over those under
  it by its mask times its opacity) stays exact.
- **A presence map** per material: per 8 × 8 mask texels, the layers whose mask is over zero in the
  block or the texel round it (one bit each, 32 at most), its levels the union of the four under
  them. The resolve reads it at the mask's level of detail and loops, in order, over the layers
  present in its tile (item 3's union), usually two to four; a layer absent from the tile costs
  nothing.
- Items 1, 3, 4 and 5 hold. Rays (the bounce, the probes, the reflections) take the material's
  folded flat colour, as now; the microblends come after the layers.

**Built (2026-11-01, the same day):**
- **The rows:** a layered material is a standard row with `RenderLayer::multilayer` (its layer
  count, where its layers' rows start, its masks' first texture, its presence map); its layers are
  UV-mapped standard rows (each its template's colour and normal maps at its own tiling and
  offset, its tint, its normal strength, its opacity as `UvMapping::alpha`), their roughness and
  metalness from single-channel maps remapped to ranges (`UvMapping::levels`,
  `metalness_texture`). The layered row keeps no map of its own in the standard slots: the rays,
  the impostors and the debug views take its flat colour as before. `GpuMaterial` 176 → 240 bytes
  (the levels, the layers' rows, the presence map and up to eight mask textures).
- **The masks:** four layers' masks to an RGBA8 texture, box-filtered levels, each texture's index
  in the row (`TextureSet::add_layer_masks`); the presence map `R32_UINT`, its levels halved as
  Vulkan's (rounded down, an odd last texel kept) and OR-reduced (`layer_presence`,
  `TextureSet::add_presence`), read one level coarser than the masks' footprint so a filtered
  sample never reads a layer the bits leave out. A row whose layers fall outside the table is
  drawn as its own colour. DDS reads BC4 (the templates' roughness and metalness). The first
  review found the levels, the margin, an assumed run of bindless slots and the bounds; all
  fixed before the push.
- **The resolve:** `multilayer_light` reads the presence bits at the masks' level, joins them over
  the wave, and lays the present layers' inputs (`uv_mapped_light`'s) front to back by mask times
  opacity: colour, normal, the lobe (by its exponent's logarithm), Fresnel, emission.
- **Measured** on the private district along the shipped game's benchmark path (DLSS Quality at
  2560 × 1440, 16:21, 1,591 lamps): 979 layered setups and 831 maps (262 MB, the game's own blocks);
  the frame 12.88 → 14.06 ms, `shading/standard` 0.79 → 1.63 ms. The walls, floors and signs take
  their tiled textures, worn patches and painted markings.
- **Next:** the microblends at the masks' edges, the masks block-compressed, texture streaming
  (#385).

**The microblends, their masks' part (2026-11-04).** The owner thought the private district's bar
ceiling was missing textures. Its maps and masks all load. What its layered surfaces lacked was each
layer's microblend: 12,377 of the district's 19,564 layers name one (concrete wear, dirt stains,
fine filth, leaks, scratches; 6,834 with a normal), the grime that breaks a layer's mask at its edge.
- **The form:** the shipped game's, as WolvenKit's Blender add-on reconstructs it from its shader
  graph: with the mask m, the microblend's alpha a (at the layer's microblend tiling and offset over
  the mesh's UVs) and its contrast c (clamped to 0.00001–1), the weight is
  `saturate(((a + m − 1)(1 − c) + m c) / c)` times the opacity. At contrast 1 the mask is as it is;
  lower, its rise becomes a ragged edge. It is zero where the mask is, so the presence bits hold.
- **The rows:** a layer's row holds its microblend in its mask slots, which a layer never uses
  (`MATERIAL_MICROBLEND`, `forge_core::material::Microblend`). Every row without one is unchanged.
- **Measured** at the bar, DLSS Quality at 1080p: 60 % of the pixels move by more than two codes,
  mostly the structure's metals and the floor (a plate behind the bar shows its diamond pattern
  through the grime); `shading/standard` within its noise (1.46 → 1.39 ms). A still camera's moving
  pixels (frames ten apart, DLSS Quality at 2560 × 1440): the bathroom 1.20 → 1.34 %, the bar
  8.25 → 8.31 %, spread over the metals and tiles the microblends reveal, which carry the room's
  reflections; softening a low contrast by the pixel's footprint changed nothing (1.34 %), so it
  was left out.
- **Their normals, the same day** (the owner: "Follow recommendations"): the microblend's red and
  green, at its strength (negative in 452 of the district's layers: the bumps inverted), bend the
  layer's normal in the frame its normal map bends in, where its mask is under 1 (eased over the
  mask's last eighth, so a mask filtered to just under 1 does not flicker), before it blends with
  the layers under it; the instance's scale taken off the frame. The microblend's bumps take no
  Toksvig widening yet. Most layers' strength is 0 or 0.01: at the bar 0.17 % of the pixels
  move, the bathroom's shimmer unchanged (1.344 %).
- **Left:** four of the district's 112 microblends, which the archives do not hold by name. The
  look is the owner's to judge on a walk.
- **A row with levels met by a ray** (2026-11-06): the rays read a metal/rough map's green and
  blue, and a row with levels has single-channel maps, whose green and blue are 0. No row with
  levels was met directly until a plain row took them, whose hits would have read no metal, a
  metal seen in a mirror as diffuse. They now read as the shading does, each map's mean through
  its levels.

## D-096 — Authored worlds cooked into packages and streamed by cell ✅ (proposed 2026-11-01 and followed under the owner's word of the night, "Follow recommendations until 10 am"; #214, #338, D-053, D-077, D-094)

**Why:** the owner, 2026-11-01, on the private district (D-083 step 4): "Zomboid uses very old
material and currently have poor performances, we have to find a good loading techniques and
optimizations, we might have to do our own package, streaming and all". The district's first
viewer holds everything resident, cooked at load: within ±256 m, 262,017 instances of 6,959 meshes
(7.1 M unique triangles, 156.5 M instanced), a cold start of 25 s (the cook 19.8 s on all cores, then
the scene 2.5 s and the BLAS 0.7 s), 16–20 s warm, 3.9 GB of VRAM (2.3 GB geometry), for one square
of a city. A game streams its world: the source game itself in 64 m sectors on seven levels, each
level's objects streamed to their own distance. Forge has the parts: cluster pages streamed by need
(#36, D-025), the cell streamer and its sources (#338: cells added and removed around the viewer on
a worker, far proxies, D-077), `#214`'s package directory with a manifest (D-053), and the research's
container (`research/memory-streaming.md` §5).

**Proposed:**
1. **A package of an authored world** (`#214`'s `packages/<name>/`, a manifest of entries with
   their hashes, versions and dependencies):
   - **meshes** as Forge cooks them (the cluster-page files of `forge_geom::cache`), content-addressed,
     each stored once however many cells use it;
   - **cells**: one record a cell of the source's grid (64 m × 2^level), holding its instances (mesh,
     place, rotation, scale per axis, material block), its lamps and decals; an object stays in the
     level its source gave it, so a tower streams from farther than a bin;
   - **materials** (the rows; the layered materials of D-095 when decided) and **textures** (#385).
2. **A source that reads it:** a `CellSource` loading a cell with one read of its record, its meshes
   counted by the cells that use them (added with the first, removed after the last; their pages
   streamed by need as #36's), its instances through the scene editor with their scale per axis
   (`add_instance_stretched`, D-083's amendment of #93's record), its lamps as the cell's set.
3. **The far cells** take the package's proxies (the source's own where it ships them, else
   D-077's merge).
4. **Reads:** plain files on the streamer's worker first, measured; the research's container (a table
   of contents over block-compressed, dependency-ordered chunks, unbuffered IoRing reads, GPU
   decompression) when the measure says the reads bind.
5. **Measured against the resident baseline** at D-094's frame: time to the first frame, VRAM,
   the worker's time per edit, and the 99th percentile and hitches (frames over twice the budget)
   along the walking, driving and flying paths.

**The private part** stays private: the converter from the source's data into the package's input.
Built in the order above, each step measured and reported; the owner can stop or change it at any
step.

**The first step, measured (2026-11-01, the private viewer, no lamps, 2560 × 1440 under DLSS
Quality, the street's end):** the source's instances bucketed by its own sector levels into the
streamer's cells (64 m to 1 km: 86 cells over the ±256 m square), added through the editor
(`add_instance_stretched`) as `CellStreamer` plans them; every mesh registered at the start, its
pages streamed through a pool; the app driving the streamer (`Running::cells`, settled before the
frame by default, `FORGE_CELLS_REALTIME=1` one edit a frame).

| | resident | streamed |
|---|---|---|
| loading screen | 2.76 s | 1.18 s |
| instances in the scene | 262,017 | 200,473 (the camera's reach) |
| GPU frame | 6.65 ms | 7.03 ms |
| clusters left to the second cull | 8.4 k | 279 k |
| VRAM | 3.6 GB | 5.0 GB (a 2 GiB pool; 1 GiB filled with the meshes' root pages) |

- **What it shows:** streaming the instances starts the district faster and draws it the same
  (the capture alike), but saves no memory yet: every mesh's root pages stay resident (6,780 pages
  of 128 KiB for 6,959 meshes), so the pool holds little else. Next: meshes counted by the cells
  that use them and removed with the last (a source's unload), and the root pages of meshes no
  loaded cell uses evicted.
- **The second cull:** a dynamic scene has no instance cells (#38), so its first pass lists far
  fewer clusters seen last frame; 279 k reach the second pass against 8.4 k: to fix before
  streaming at a city's scale.
- **The lamps, found on the way:** the district's 11,150 static lights as Forge's lamps (512 a frame
  at most, every one tracing shadow rays) cost about 35 ms by day: `shadow/lamp rays` 9.6 ms, the
  shading's lists 9.8, the probes' and the bounce's lamp-lit hits 9.5 and 8.6. The source casts
  shadows from few of its lights (its raster mode's local shadows on a twelfth of them): a lamp's shadow made optional, and
  many lights sampled (RTXDI, D-065's amendment) rather than listed, before the night. Measured the
  same night at midnight (6,640 lamps): the lists 43.4 ms, ReSTIR 28.0, ReSTIR with the GI rays'
  hits sampling their lamps (#386) 13.7.
- **The pages packed (the same night, #387):** a static scene's small meshes (at most 64 KiB over
  up to eight pages) now share pages: the district resident 12,539 pages → 2,436, VRAM 3.64 →
  2.38 GB, the same frame. A dynamic scene packs too, its editor never freeing the shared pages
  (`owned_pages`): the streamed district holds 1,932 pages for its view, a quarter of a 1 GiB pool.
- **Streamed in real time** (one edit a frame, `FORGE_CELLS_REALTIME=1`, a 256 MiB pool, packed
  pages, no lamps, DLSS Quality, 840 frames at a fixed step): still p50 6.96 / p99 15.0 ms, flying
  40 m/s 5.89 / 14.3, driving 25 m/s 5.94 / 14.0; the pool full without wanting a page; loading
  2.2 s; VRAM 2.97 GB. Against the same square resident (packed: 2.38 GB, p99 8.7–9.2 ms) the
  stream is a worse deal at this size: its tables reserved for the dynamic scene, a top-level
  structure rebuilt each edit, the second cull without instance cells. It pays where the world is
  larger than what is seen; next: those three, then a district larger than the square.
- **The second cull, explained (the same night):** not the dynamic scene but instance occlusion's
  auto mode (#38), which turns on from 65,536 hidden instances: the streamed square holds 200 k
  instances and hides 47 k, so it stayed off and every visible instance's clusters reached the
  second cull. Forced on (`--instance-occlusion on`): 8.3 k clusters left to it as resident, the
  still frame 7.29 → 6.80 ms, the flight's 3.5 ms. The p99 stays near 14 ms along the paths
  (13.7–14.0): the stream's edits, not the cull, set its worst frames; to profile next. The auto
  threshold suits the static scenes it was measured on; a dynamic scene's is to measure.
- **The edits' worst frames, explained and fixed (the same night):** not the top-level structure
  (without rays the flight's p50 and p99 stayed 8.4 ms apart, 2.16 / 10.6) nor the frame thread
  (0.02 ms to apply an edit) but the edit's copies (`scene/edit`, on the copy engine, which the
  culls wait for): one region per changed instance record, 3,500–36,700 when cells came in, about
  0.5 µs each, 3–19 ms of GPU on the frames that published a loaded cell. A copy now folds into
  the one before when it continues it at both ends (`forge_gpu::commands::push_copy`, and
  `write_buffers_staged`, the worker's records for the structure): a cell's new instances, in
  consecutive slots, copy as a few regions. The flight at 40 m/s: p50 5.77 / p99 13.7–14.0 →
  5.79 / 7.65 (two runs: 7.65, 7.68), no frame's GPU over 8 ms; the worker's longest edit 47 →
  23 ms.

**Amended (2026-11-07): meshes come and go with the cells that use them.** This closes the first gap above (the owner moved
the effort to cell streaming).

- **The unload hook:** `CellSource::unload(cell, editor)`, called by the streamer once a cell's instances and its own meshes are
  out. It does nothing by default.
- **`SharedMeshes`:**
  - a source acquires a mesh by its key for a cell; the editor adds it with the first cell that asks for it;
  - on unload, the source releases the cell's meshes and takes out those no cell holds any more.
- **Measured:** the private district streamed along its path in real time (one edit a frame, 2560 x 1440, DLSS Quality),
  two alternated runs each:

  | | Every mesh registered at the start | Meshes with their cells |
  |---|---|---|
  | Frame | 15.45, 15.24 ms | 14.39, 14.38 ms |
  | p99 | 18.66, 18.62 ms | 17.48, 17.50 ms |
  | Geometry | 5,397 MiB | 4,395 MiB |

  That is 1.0 GB less and 1 ms less a frame.

**Fixed with it (the owner's catch): a dynamic scene's cut-outs drew solid.** Such a scene decided whether it needs the
cut-outs' raster from the instances it had at its start. A streamed district has none then, so its grilles, fences and leaves
drew solid. It now takes that raster whenever its table has a cut-out row.

## D-097 — A rasterised mode lit by precomputed transfer: relit probes in sparse bricks ✅ (proposed 2026-11-01; the owner's go the same day; D-094, D-095, D-096)

**Why:** D-094's amendment holds Forge to the shipped city game's benchmark mode for mode, its
rasterised mode included (152.3 fps, 6.48 ms p50 under DLSS Quality at 2560 × 1440), and the owner
asked for precomputed lighting ("a good technology to have, it avoids a lot of real-time
computations") while the sun, the moon, the weather and the lamps still change. The shipped game
does it: about 22,000 precomputed GI cells of 64 m (a sparse brick structure, ~12 GB), reflection
probes with per-face depth and a sky flag, and a list of far lights. The research
(`research/baked-gi-time-of-day.md`) compared precomputed radiance transfer, relit probes, keyed
bakes, Enlighten-style form factors and probe volumes.

**Proposed (the research's recommendation):**
1. **Relit transfer probes in sparse bricks** (The Division's design, GDC 2016), one file per 64 m
   cell in D-096's package: probes in 4 × 4 × 4 bricks at 1, 3 or 9 m, each with its sky visibility
   (L1) and its factors to bricks of surfels; the surfels' position, normal, folded albedo
   (D-095), a wet or snow weight; the cell's lamp list and a far-field record. About 0.6–0.85 MB a
   cell (estimate).
2. **Baked by Forge's own GPU path tracer** (no form factors in the bake: ARM's US 10,672,177;
   rays, not depth buffers over directions: Microsoft's US 7,948,490).
3. **Relit each frame:** the surfels lit by the live sun, moon, sky and lamps, averaged into
   bricks, summed into the probes round the camera (3 × 3 cells, farther ones staggered); about
   0.5–2 ms on the RTX 5070 Ti (an estimate from The Division's 0.47 ms on a GTX 760, to measure).
   Nothing keyed by the hour: night, weather and lamps cost no data.
4. **Relit reflection cubes** (albedo, normal, depth, sky bit), normalised by the volume, after.
5. **Measured first on one cell** of the private district against Forge's path-traced reference at
   a fixed exposure, at noon, dusk and night with its lamps.

**The decision the owner takes:** Imagination's US 10,672,183, US 11,127,198 and GB 2571656 (live
to 2036) update a probe "from a previous frame" with rays' lighting contributions; a relight that
sums brick light by factors traced at bake time may read on them. Recomputing each probe from
scratch every frame (no blending with the last) keeps further away; the fallback is Far Cry 3's
pure transfer (2012, before their 2015 priority), which loses the lamps' bounce. Until the owner's
word (or counsel's), nothing of the relight is built.

**Decided** (the owner, 2026-11-01: "Yes and can use or test division's design or improve upon it if
needed"): the relight is built, under the owner's ruling of 2026-10-17 on the same Imagination
family for Forge's ray-traced probes ("live with it for now"); each probe still recomputed from its
bricks every frame, the farther from "from a previous frame". The Division's design is the start,
to be measured and improved where Forge's own calibration (its path-traced reference) shows it
falls short. The vendors' kits were checked first: NVIDIA's SHaRC and NRC and AMD's FSR Radiance
Caching all trace every frame (AMD's is DirectX 12 only for now), so none is precomputed; AMD's
Brixelizer GI (FidelityFX SDK 1.1.4, MIT, any GPU) becomes a column of the GI comparison for the
traced modes.

**Step 1 measured (2026-11-01, #388): relit precomputed hits on Forge's own probes.** A prototype
behind `FORGE_GI_BAKED=1`: each settled probe's rays keep fixed directions; the first time, each
ray's hit is kept (position, normal, albedo, emission, distance, and 64 bits of open sky along
fixed upper-hemisphere directions, 64 B a ray, 280 MB for the private district's 34,560 probes);
every later frame it is relit with no ray (the key light through the bits, the sky, the probes'
bounce, the lamps without shadows) into the probes' blend as before. The cache holds no light, so a
bake at any hour is the noon bake relit. On the benchmark district's street, 1280 × 720, against
Forge's path (`r.gi.reference=4`), mean |log2| over 16 × 9 tiles, the per-pixel bounce off for the
probes:
  - noon: baked probes 0.056 (frame ×0.996), traced probes 0.043, the default (probes and the
    per-pixel bounce) 0.034;
  - dusk (19:30): 0.041 (×0.995), 0.033, 0.025;
  - night: every technique 1 to 1.6 stops under the path (the lamps' known difference, #369, the
    calibration's reason to run its path without them): no verdict.
  The baked probes lose 0.01 against the traced ones, with no ray at run time. Their cost at dusk
  with 1,591 lamps, DLSS Quality at 2560 × 1440, 3,000 frames, two runs each: `gi/probe rays`
  1.94–2.06 → 1.34–1.38 ms, `gi/probe blend` 0.68–0.77 → 0.54–0.55, the frame 10.5–10.8 → 9.9–10.1;
  the relight's own cost is the lamps' weighing and the bounce's lookups, which step 2's bricks
  (about 24 a probe, not 96 rays) cut.
  Its known gaps, left for step 2's format: jelly (the jelly pass's surfaces) is not in the kept
  hits, and the cache is not cleared when the scene's geometry changes or the probes reset (a
  probe's key is its place, so only a moved probe re-traces). Off, every capture of eight scenes is
  unchanged (0 px).

## D-098 — Glossy reflections: rays inside the lobe, denoised, under the sharp mirror's band ✅ (proposed 2026-11-04; the owner's go the same day: "follow recommendations"; on hold: the speckle was a rounding fault, below)

**The need** (the owner, 2026-11-04, the private district's bar: "some surfaces have issue … mirrors
and bin … the bar door"): the layered metals (a door, a bin, a bathroom's metal mirror; roughness
0.30–0.42, normal maps tiled over them) showed a speckle of every colour, steady with the camera
still. Measured at the owner's view, the lamps off, DLSS Quality:
- with the mirror rays off (`r.reflections.rays=0`) the metals are clean: the speckle is what D-050's
  rays bring back;
- the rays meet the right surfaces (their hits' albedo alone draws the room as a clean mirror image);
  their light differs from hit to hit, and the layers' normal maps aim each pixel's ray apart
  (flat materials give smooth bands instead);
- neither the motion vectors moved to the reflection (`r.reflections.motion=0`), the reflections'
  history (`--no-reflection-history`) nor DLAA change it: it is not a temporal failure;
- two causes of the hits' light were mended in the district's loader: the street's probes lit the
  bar's surfaces (the scene had no rooms: the source's interior volumes are now the probes' rooms,
  D-068, and the bathroom is dark with its lamps off), and the source's BC5 normal maps kept unit
  normals in their mips (now rebuilt as Forge's own, D-086, so Toksvig widens the tiles' grout).
  The speckle on the metals stays: most of their maps are BC1, and up close Toksvig is small anyway.

**Why:** D-050 traces one exact mirror ray a pixel, all of it up to roughness 0.30 and fading out
by 0.42 (#306), and steadies it with a history that reprojects a flat mirror. A brushed metal at
0.35 reflects over a GGX lobe some 10° wide each side; a sharp ray shows each ripple of its normal
map instead, and the room's light varies enough between neighbouring hits to read as noise. The
game's ray-traced mode draws these as soft reflections.

**Proposed** (D-050's option (c), "both"):
1. **Two bands.** The sharp mirror as now for the smooth rows (roughness under about 0.15: glass,
   water, mirrors, paint); for the glossy band (0.15 to 0.45), one ray a pixel aimed inside the
   row's GGX lobe (visible-normal sampling), its hit lit as today's mirror hits are.
2. **Denoised by NRD's REBLUR_SPECULAR** (already in the frame for the lamps' and the bounce's
   signals, D-065, D-088; on every vendor), fed the ray's hit distance and the pixel's roughness;
   it reprojects by the virtual point as D-050's history does, and widens its blur with the lobe.
   D-050's history keeps the sharp band.
3. **Past 0.45** the probes' dimmed sky, as today.
4. **Checks:** the bar's metals at the owner's view against the game's grab of the same place; the
   village's glossy floor (#306's) and the city's glass unchanged in the sharp band; the cost in
   `docs/PROFILE.md` (NVIDIA's figure for REBLUR's specular alone is about 1.2 ms at 1440p on an RTX
   4080; the glossy band's tiles only).

**Also seen, separate:** the source's capsule lights become spheres of half their length (up to
0.5 m), so a mirror shows a 1 m glowing ball where a tube hangs; and its 1,363 area lights given in
nits are read as lumens until calibrated against its pictures (the private district's notes).

**For the owner:** take (c) as proposed, or keep the sharp mirror and narrow #306's fade (fewer
rays on glossy rows: no speckle, but no reflection of the room either, the probes' sky instead).

**Decided** (the owner, 2026-11-04: "follow recommendations"), the band drawn by the surface rather
than by roughness alone: a row without a normal map (glass, water, a mirror, paint) keeps D-050's
exact mirror ray at any roughness, so the city's glass stays as chosen; a row with a normal map or
layers (D-095) aims its ray inside its GGX lobe, over the band #306 traces (to roughness 0.42), its
history reprojected with the surface rather than the mirror's virtual point (which the bumps break),
then NRD's REBLUR_SPECULAR where NRD is installed. Built in two steps, each measured.

**Found the same day, after the proposal: the lobe alone does not mend it.** Aiming the rays of the
0.30–0.42 band inside their lobe (a patch kept in the private district's notes) left the bar's
metals as they were. Each term of the hits' light taken alone, at the owner's view, the lamps off:
their places (distance), normals and albedo all vary smoothly from pixel to pixel, the rays staying
in the room (3 to 8 m); the sky's irradiance at them is smooth; the probes' irradiance at them is
the speckle, with the hit lookup (`probe_irradiance_hit`), with the pixels' lookup
(`probe_irradiance`), and with a fixed upward normal alike, the room's own probe grid (D-078) or
the cascades. So the probe field's light varies from point to point there, by far more than its
1 m grid would explain: most likely the probes' visibility (the Chebyshev test on their depth
moments) switching between probes whose light differs (one seeing a lit doorway, one a dark wall).
The surfaces seen directly hide it, lit near the camera by the per-pixel trace (D-088); the probes
alone show the room dark, as it is, at the camera's distance. The district's rooms (its interior
volumes as D-068 regions, cut into rectangles so each grid stays in its room) and its normal maps
rebuilt as D-086's did not change it. Next, before the lobe and REBLUR: the probes' visibility
in that room, and a short trace from the mirror hits as the pixels have.

**Found later the same day: the speckle was the colour image's rounding, mended apart from D-098.**
The finding above was wrong: those captures of the probes' light went through the same fault. Each
term written straight into the frame instead (`reflections_main`'s output, the anti-aliasing and
the history off), the hits' probes, albedo, metal and sun are all smooth, and the speckle is red,
green and blue apart, each channel its own: rounding, not light. `sky_reflection` shaded a traced
row with the open sky in its mirror direction, at the ray's weight, and the mirror rays' pass took
that sky off again as it added the hit (`request.w * tint * (hit - sky)`). In a dark room the sky
is hundreds of times the room's light, and the class pass had stored it in the half-float colour
image (11 bits): its rounding was as large as the reflection, left behind when the sky came off.
Negative halves clamped, it also brightened every reflecting metal indoors. Mended by leaving the
traced share out of the shading (the pass adds the hit, or the sky on a miss: the same sum without
the cancelling); the jelly, whose ray is traced in its class pass in registers, adds the sky back
where it subtracts it. At the owner's view (the door's lower panel; the difference from a 3 × 3
median over the local light): 0.141 → 0.063 with DLSS Quality and the lamps on (the rest is the
lamps' single shadow ray at the hits, which DLSS mostly averages), 0.396 → 0.027 with neither
anti-aliasing nor lamps; the door's mean light 12 times lower, its true reflection of a dim room.
The village's map views change by under 0.003 (outdoors the sky and what the rays meet are alike),
the Bistro's day view in 98 pixels of 1.44 million (ꟻLIP mean 0.0005). The lobe and REBLUR (step
1 kept as a patch in the private district's notes) are now a look question, not the speckle's
answer: on hold until the glossy metals are compared with the game's grab of the same place.

## D-099 — Fog lit by the lamps: authored fog volumes in D-032's froxels, the lamps' light scattered ✅ (proposed 2026-11-04; the owner's go the same day: "Follow recommendations"; D-032, D-065, D-094, #386)

**The need** (the owner's comparisons with the shipped city game's benchmark, the private district's
bar): the game fills the bar with a haze that every lamp lights. The pool table's red light, the
neon's green and the lamps' pools all glow through it. Forge draws the same room clear, its lamps
lighting only the surfaces. It is the largest look gap left in the bar, after the layers'
microblends (D-095).

**What the authored data holds:** 223 fog volumes in the district (`worldStaticFogVolumeNode`), each
a box with:
- a density (mostly 0.005–0.1 a metre, a few at 2–5);
- a height falloff (on or off) and a falloff;
- a colour (186 of 223 white);
- an absorption, an ambient scale, a priority and a blend falloff.

The bar's own volume is a 31.8 × 9.0 × 24.3 m box: density 0.0175, falloff 0.15, absorption 0.4,
ambient 0.6. The colour comes from the lamps, not the fog.

**What Forge has:** one froxel volume (D-032, 160 × 90 × 64), lit by the sun through a shadow ray a
froxel, its medium from noise; the far field the atmosphere's aerial-perspective table.

**Proposed:**
1. **The medium from authored volumes:** boxes (an inverse transform, a density, a height falloff
   from the box's floor, a colour, an absorption), the highest priority over the others where
   they overlap, blended at their edges. The scenes' own fog: a scene with none keeps D-032's look
   unchanged.
2. **The lamps' light scattered:** per froxel, the lamps whose reach meets its column of the
   frame's grid (`lamps::NearLamps`, #386), weighed as the light lists weigh a surface (the same
   reach, beam and colour), with an isotropic or mildly forward Henyey–Greenstein phase.
   - **First without shadows:** a lamp lights the fog of its room through walls only where its reach
     spans them, as the source's raster mode does.
   - **Then shadowed, as an option:** one shadow ray a froxel to a lamp drawn by weight (as the
     hits' lamps are), steadied by D-032's temporal reprojection.
3. **The sun as now,** and the ambient light from the probes' irradiance at the froxel times the
   volume's ambient scale (rooms dark, the street lit).
4. **Applied as D-032's,** to every pixel by its depth; reflections and glass take the volume at
   their hit later.
5. **Measured:**
   - the bar's view against the game's grab of the same benchmark frame (light ratio maps, as D-083's
     comparisons);
   - the cost on the benchmark path (a target of about 1 ms at DLSS Quality, 2560 × 1440);
   - a still camera's shimmer;
   - the village and the Bistro unchanged where they have no volumes.

**Patents:** none surfaced in a first web search. The froxel volume has been published since 2014
(Wronski, SIGGRAPH; Hillaire, 2015), and open engines ship it (Godot 4's fog volumes, Bevy's). A
proper search of the patent databases on lights scattered in a froxel volume runs before any of it
is built (research, D-008's practice).

**For the owner:**
- go or no-go;
- the lamps' fog unshadowed first (cheap, the raster mode's look) or shadowed from the start (the
  ray-traced modes' look, a ray a froxel).

**Decided** (the owner, 2026-11-04: "Follow recommendations"): as proposed, the lamps' fog
unshadowed first, the shadow ray a later option; the patent search before the build.

**The patent search (the same day, web research: Google Patents' full text for "froxel", 22 hits
read, then by holder; Espacenet and others refused automated requests; not legal advice):** no
live patent found reads on items 1–4 as written.
- **Sony's pending US 2026/0038215 (EP 4687106, GB 2643037)** samples fog on a grid, then builds a
  finer second set by interpolation. Wronski 2014 predates it. Forge reads its volume once a pixel,
  trilinearly: no finer intermediate grid, ever.
- **NVIDIA's ReGIR (US 12,299,801)** fills grid cells with lights by resampling. The columns list
  every lamp in reach, chosen deterministically: never fill them by RIS.
- **NVIDIA's pending US 2023/0419590** steers light sampling by values learned from earlier rays:
  the weights stay analytic.
- **AMD's tiled lists (US 10,803,655, US 12,136,165)** are per screen tile; Forge's are world
  columns.
- **Arm's US 10,665,010** keeps caster sets per lit sub-volume: none here (shadows trace the TLAS).
- **Imagination's family (US 10,672,183, US 11,127,198, GB 2571656)** updates a directional
  representation from the previous frame: a froxel holds one colour, the phase applied for the
  view, never lobes blended over frames.
- **Prior art:** Wronski (SIGGRAPH 2014), Hillaire (2015, local lights in froxels and their
  temporal reprojection), Unreal 4.16 (2017), Unity HDRP's density volumes (2018), Godot 4's fog
  volumes (MIT, 2022).
- **Before the shadowed option:** run the full-text queries again (the site blocked the later
  ones), since applications from the last 18 months are not yet published.

**The shadowed option's patent search (2026-11-05, web research, not legal advice):**
- **The question:** the lamps' fog shadowed, either by one ray a froxel to a lamp drawn by its
  unshadowed weight, or by a ray to every lamp above a weight. Both are steadied by D-032's
  reprojected history.
- **Where it searched:**
  - Google Patents answered for "volumetric fog" (priority from 2019, 82 hits, read by title)
    and for "volumetric fog" + "shadow ray" (one hit, NetEase's CN115591235A, a model-rendering
    patent), then refused every request.
  - The EPO's data service, claims mirrors and about 30 web queries filled in. FreePatentsOnline,
    Justia, Patentscope and Espacenet's API refused automated requests.
- **What it found:** no live patent read on either form. The nearest:
  - **NVIDIA's light importance caching** (US 11,756,254 B2 and US 12,374,024 B2, granted, to
    about 2040; the latter is the "pending US 2023/0419590" above) claims choosing rays per
    region, a non-cubic voxel among them, from values that earlier rays found. Forge's weights
    are analytic, never learned from rays.
  - **NVIDIA's ReSTIR family** (US 11,948,246, US 12,361,635 B1, ReGIR US 12,299,801) resamples
    reservoirs and reuses the previous frame's samples. A fresh weighted draw each frame does
    neither.
  - **NVIDIA's spatiotemporal blue noise** (US 11,727,621 B2 and continuations, to about 2041)
    builds its masks by importance sampling and swaps. A hash, or a classic blue-noise tile,
    stays clear.
  - **NVIDIA's shadow denoisers** (US 11,600,036 B2, US 11,727,535, US 10,740,954) set their
    filters from the shadows' temporal variance. The volume's plain exponential history does not.
  - **Disney's "Distribution caching for direct lights"** (US 9,619,922 B2, to about 2034)
    caches light lists, distributions and a visibility list at points. Its claims could not be
    read.
  - **Not relevant:** Sony's EP 4700708 A1 and its US family (2024–2025) upscale froxel grids by
    learning and claim no lighting.
  - **Chinese filings** (Tencent, NetEase, Perfect World, Sichuan University on soft shadows in
    volumetric fog): titles only; they matter for sales there.
- **Prior art:**
  - Ward's adaptive shadow testing (1991): the lights in order of contribution, the small ones
    skipped, as the threshold form.
  - Shirley, Wang and Zimmerman (1996): one light by its unshadowed contribution, one ray.
  - Wronski (2014) and Hillaire (2015).
  - Unreal 4.16 (2017): froxel fog with shadowed lamps and a reprojected history.
  - AMD's stochastic light culling for media (2016, 2022).
  - Heitz et al.'s ratio estimator (2018).
  - RTX Remix 0.1.0 (MIT, 2023): one visibility ray a froxel to a chosen light, accumulated over
    frames.
  - Unreal 5.5's MegaLights (2024).
- **The rules a shadowed fog keeps:**
  1. The weights analytic and unshadowed, every frame: no visibility cache, no learned
     importance, nothing fed back from the rays.
  2. No reservoirs, no RIS, no reuse of a neighbour's or the last frame's lamp.
  3. The history blends radiance alone: no variance-driven filter, no penumbra masks.
  4. The random numbers from a hash (PCG) or a classic blue-noise tile shifted by the golden ratio
     each frame.
  5. The lists per world column, every lamp in reach: no distant-light distributions, no
     visibility lists.
- **The threshold form is the clearer.** It is deterministic, older than every claim found, and
  needs no random number.
- **Before shipping the single-ray form:** read the claim 1 of Disney's US 9,619,922 and the
  granted claims of US 12,374,024.

**Step 1 built (2026-11-04): the boxes and the lamps, unshadowed** (`forge_render::FogVolume`,
`shaders/fog.slang`, the city app's `Running::fog_boxes`, `--no-fog`).
- **The volume:** D-032's layout (160 × 90 × 64 froxels to 64 m, quadratic). Its medium is the
  boxes at each froxel laid in priority order (ties by their order in the scene), each over those
  under it by its edges' fade (metres to its nearest face, per axis). It thins with height above
  the box's floor, and the share absorbed comes from the box. The 32 boxes kept are the nearest by
  their bounding spheres, so a large box round the camera is never dropped.
- **The light:** the lamps of the froxel's column of the lamps' grid (#386, every lamp beyond the
  grid), weighed as the light lists weigh a surface facing each (reach, beam, colour), plus the
  sun through a shadow ray, with a Henyey–Greenstein phase (g 0.2). Applied over the HDR scene
  before the exposure meter and the upscaler.
- **Each froxel lit at its middle,** a lamp's light taken from no nearer than half the froxel's
  size. Jittered within the froxel each frame (as D-032's dust is), the volume shimmered with nothing
  to settle it: a still camera's moving pixels in the private district's bathroom went 1.50 → 2.81 %,
  and a sample landing beside a bulb lit its whole froxel white. At the middle: 0.23 %.
- **Measured** on the private district, DLSS Quality at 2560 × 1440, 25 boxes in reach: the bar
  `fog/light` 0.56 ms (jittered, before the fix), the bathroom 0.30; `fog/apply` 0.06–0.07,
  `fog/integrate` 0.001.
- **The look:** at the data's densities (`densityFactor` taken as extinction per metre) the haze is
  faint. Five times thicker, by eye against the game's frame of the same benchmark moment, it fills
  the bar and glows round its lamps; the private viewer's default until the environment's own fog
  is read.
- **The lamps' shares of the fog, the same evening** (the owner: "Follow recommendations"):
  - **Each lamp now carries the share of its light the fog scatters** (`Lamp::fog`, the source's
    `scaleVolFog`). 2,230 of the district's lit lamps take none, and they had lit the haze.
  - **The source's lights for its fog alone** (`sceneDiffuse` 0, 620 at a full share; a green tube
    over the bar among them) light the fog as a list of their own (`Running::fog_lamps`, the
    nearest 256 whose reach meets the volume; up to 82 at once in the district). They had been
    left out with the GI-only lights.
  - **Measured at the bar:** `fog/light` 0.65 ms with 211 of them; a still camera's moving pixels
    in the bathroom 0.44 % (0.23 % without them, 1.50 % without fog).
- **The environment's own fog, the same evening:**
  - **The source.** The shipped game's master environment holds a volumetric fog everywhere: density
    0.003 a metre, even up to 125 m and thinning by 0.01 a metre above, absorbing 4 %. It lets
    0.75–1 % of the sun's light into the fog (`globalLightScale`), all of the lamps'
    (`localLightScale` unset), with a phase of g 0.8.
  - **In Forge.** These are `GlobalFog` and `FogParams::light_scales`, and the scenario's
    `Running::fog_environment`. The boxes lay their medium over that fog.
  - **The look.** With every value the data gives, the bar's haze is fainter than the game's. Some
    factor is not in the extracted settings: perhaps the lamps' fog scale has a default other than 1,
    or the neon's own emission lights the fog. The private viewer keeps its boxes five times
    thicker, by eye.
  - **The cost.** The fog everywhere puts every froxel in the lamps' loop: `fog/light` 0.65 →
    1.21 ms at the bar, to bring down later (a coarser volume where only the global fog is).
  - **Its limits (the review).** The volume ends 64 m from the camera, so a facade at 200 m and the
    sky take only 64 m of the fog everywhere (the game's reaches 125 m and beyond). A box of no fog
    cuts a hole in it (the source's `{F_fog_zero}` volumes). The values are clamped on upload.
- **Left:**
  - the glow the neon's own emission gives the game's fog (only lamps scatter here);
  - the ambient light (the volume's `ambientScale`);
  - the lamps' shadows (the patent searches first);
  - reflections and glass seeing the fog.

**The probes' light in the fog (2026-11-04, item 3).** Each froxel scatters the probes' light every
way alike: a white surface facing up and one facing down take irradiances E_up and E_down there
(`probe_irradiance_hit`, in the froxel's region: a room's grid in a room), its mean radiance taken
as (E_up + E_down) / 2π: exact for light even over each half of the sky, light from straight above
counted twice, light from the side (a sunlit wall's) less than it should. It carries the sky's and
the scene's bounce, glowing surfaces' included, which the lamps' direct light leaves out. Without
probes, the open sky's light, as surfaces take it. The extinction that scatters it (the extinction
times its share) is blended across a box's edge as one: blended apart and multiplied, a box's edge
took a shell 3 to 5 times too bright at night.
- **Its share.** `FogParams::ambient` holds two shares: one for the fog everywhere, one for the
  boxes, where it multiplies each box's own `FogBox::ambient`. The shipped game's environment lets
  in a twentieth by day (`ambientScale` 0.05) and twice over at night, and 0.15 in its volumes
  (`localAmbientScale`); its boxes' own shares are mostly 1 or 0.1–0.5. At share 1 everywhere, the
  district's sunlit streets came out 0.8 EV too bright against the source's frames; with the
  source's shares they are within 0.1.
- **Skipped** where a froxel's share times its extinction is under 2·10⁻⁴ a metre: over the
  volume's 64 m that is under 1.3 % of the radiance. By day the district's fog everywhere falls
  under it.
- **The cost** (`fog/light`, the district at 1600 × 900): as first pushed, the bar by day 1.89 →
  2.41 ms, at night 3.03 → 3.62, the street by day 1.34 → 1.77. With the extinction and its share
  blended as one (fewer froxels over the threshold) and the lookup in a branch of its own (chosen
  per direction inside the loop, it cost the bar's night 1 ms more), the bar by day 2.12, at night
  3.30. Two lookups a froxel in a rolled loop.
- **The look:** a faint lift of the haze, mean ꟻLIP 0.020 (the bar by day), 0.027 (at night),
  0.028 (the street). The neon's own glow in the game's haze is still missing: the probes stand
  1.8 m apart in the bar's hall and blur it. `--fog-ambient 0` turns it off.

**The lamps' phase apart from the sun's (2026-11-04).** An authored environment's anisotropy is
its global light's (the shipped city's `globalLightAnisotropy`, 0.8), and it was the fog's for
every light. At 0.8 a lamp seen through the fog scatters 45 times its light scattered every way
alike towards the camera: each strong lamp of the district's bar lit a ball of haze round it (a
green one at the neon, a blue one at the bull's skull). The lamps now take their own anisotropy
(`FogParams::lamp_anisotropy`, Forge's 0.2), the sun keeps the environment's. The bar's haze is
even and warm as the source's frame shows it: in the benchmark's frame 21 its colour reads red over
green 1.31 and blue over green 0.55 (the source 1.20 and 0.41; with 0.8, 0.93 and 0.61). The
village and the Bistro have no fog volumes and are unchanged.


**The lamps' shadows in the fog, the option (2026-11-05, `FogParams::lamp_shadows`, the city
app's `--fog-lamp-shadows N`, off by default):**
- **The form:** the patent search above found the threshold form the clearer, and it is built.
  At each froxel, its strongest lamps are tested by a shadow ray each. There are up to N of them
  (at most `FOG_SHADOW_RAYS`, 4), each a tenth or more of the froxel's lamps' light. The rest
  stay unshadowed: the weaker lamps, the lamps of the fog alone, those casting no shadow.
  - After Ward 1991's adaptive shadow testing, the strongest lights tested: deterministic, no
    random draw, and no history needed.
  - The weights are the lamps' unshadowed light, never learned from rays.
- **The clearance:** a ray ends 30 cm before the lamp's sphere (`FOG_SHADOW_CLEAR`). Ended 5 cm
  short, the glass under a pendant's bulb met every ray from under it and put out its cone. Within
  that distance of a lamp, a wall lets its light into the fog behind. A flat piece's ray ends 2 cm
  short of its middle, as the surfaces' do: a curtain over a panel shades its haze (#370).
- **Off:** the lamps' sum is computed as before and the shadow block is skipped. The bar's frame
  without anti-aliasing (1600 × 900) differs from HEAD's shader on 4 of its 1.44 million pixels,
  by one code value (the compiler's work moved; two runs of the same shader are 0 px apart). Its
  `fog/light` fell from 0.707 to 0.504 ms (0.708 and 0.502 again; the frame 13.35 → 13.14 ms), the
  pass compiled differently; why is not established.
- **Seen in motion:** the fog has no history, so a froxel is shadowed whole or not at all, on
  the 160 × 90 × 64 grid fixed to the view. Shadow edges near lamps may crawl as the camera
  moves.
- **Cost:** the private district's benchmark path (2560 × 1440, DLSS Quality): `fog/light` 0.449
  → 0.497 ms, the frame 14.49 → 14.52 ms; its bar's views +0.05 to 0.07 ms.
- **The look:** the pool table's lamp throws its table's and stools' shadows into the haze. The
  haze leaking through walls is gone.
- **The source's six interior frames (geometric means, EV against the source's):**
  - the pool corner +1.66 → +1.27, the next view +0.81 → +0.69;
  - the landing −0.96 → −3.65 and −0.76 → −2.37.

  The landing had been lit by red haze through its walls. The source, path traced with shadowed
  fog, keeps it bright, so the landing lacks light of its own, which this shows. **Off by
  default** until the landing's light is found. The village and the Bistro have no fog volumes.

**The history (D-099's amendment, 2026-11-05, `FogParams::history`, on; `--no-fog-history` the
A/B).** The owner, on the bar's haze: "Fog is not good, pixelized and not stable". Lit at their
middles, the froxels showed as 16-pixel blocks along the lamps' cones, and the blocks crawled as
the camera moved.
- **The form** (Wronski 2014, Hillaire 2015, Unreal 4.16's volumetric fog; Godot's and Flax's do
  the same):
  - each froxel is lit at a point jittered within it each frame: the R2 sequence (Roberts 2018)
    plus a pcg3d offset of its own, so neighbours sample different places;
  - the result is blended into last frame's volume where the froxel's middle stood then,
    keeping 0.9 of it (about ten frames' samples);
  - the reprojection goes through last frame's view-projection; the slice is found by inverting
    the light's sample distances, then read bilinear within two slices and linear between them;
  - the history is brought to this frame's exposure (the CPU's, last frame's kept beside it).
- **Where there is no history,** a froxel is lit at its middle, unjittered, as before:
  - the first frame, or a frame after one without fog;
  - a cut: the anti-aliasing's reset, the camera moved more than 4 m or turned more than 30° in a
    frame (a teleport, the scene's origin moved);
  - a point outside last frame's volume;
  - a history value that is not finite (a fault kept in a history would never clear).
- **The pass:** `fog/light` writes the history alone and `fog/integrate` reads it. The froxels'
  own transient is gone, as is the buffer of exposures the first form read and wrote.
- **The look:** the cones' edges are smooth, and their blocks no longer crawl. A still camera's
  shimmer is unchanged: the bar 0.71 → 0.75 %, its landing 0.18 → 0.20 % (pixels over 2 codes,
  frames ten apart).
- **The cost:** on the district's path (2560 × 1440, DLSS Quality, interleaved runs) `fog/light`
  0.43 → 0.46–0.47 ms (its zone, beside the particles' draw). The frame stays within the runs'
  spread: 14.80 and 15.73 ms without, 15.62 and 15.95 ms with (one run without had the lamps'
  ReSTIR 0.3 ms faster too).
  - The zone shows less than the pass costs. With no barrier between them, the pass runs beside
    the particles' draw before it, and its zone counts only what comes after that draw: 0.47 ms on
    the district's street at 1280 × 720, 0.91 ms with no particle alive.
  - **Tried and left out:** `fog/light` on the async compute queue. It needs this frame's movers
    and probes, not its scene, but there it shared the GPU with the lamps' ReSTIR and the path took
    longer.
- **The owner's "the fog pops":** in the street it was the steam (D-064's amendment, the near
  fade). At the bar's far end, the jump at the benchmark path's 16.8 s was the camera turning: a
  lit stretch of haze came in at the screen's edge.
- **Patents:** none found in a search (2026-11-05). The method has been public in shipped engines
  since 2014 and in open source since (Unreal, Godot, Flax).

**The cull (the same day, `FogParams::cull`, on; `--no-fog-cull` the A/B).** No pixel reads the
froxels that lie behind every surface in view, so they are not lit.
- **The bound:** `fog/tiles` takes each froxel column's farthest surface from the depth (the sky's
  infinite). `fog/bound` turns it into the last slice lit: two past the farthest of the column
  and the two columns round it each way.
  - `fog/apply` filters across one column each way and between two slices.
  - A history read reaches a slice on and, shifted by the jitter, a column further. With the
    eight neighbours alone, a froxel by a silhouette lost its history every other frame (the
    review).
  - A froxel past that slice is skipped.
- **The history:** a skipped froxel is marked (an extinction of −10,000, which `fog/integrate`
  takes as empty). A history read that reaches a mark at any weight is refused, so a froxel
  uncovered as the camera moves is lit afresh.
  - **Tried and left out:** carrying a skipped froxel's history forward instead. Where the bar's
    far end came out from behind a pillar, its haze stayed dim for frames: 4.8 % of the frame over
    2 codes, up to 65. Marked, with the bound above, 0.67 %, up to 29, and it looks the same.
- **On a still camera:** 1–2 codes on about 1 % of the pixels. More than 2 codes on 27 pixels in
  the bar, 203 in the street. The shimmer is the same: the bar 0.937 % against 0.936 %, the street
  11.13 % both (its steam moving; measured with the eight-column bound).
- **The cost:** `fog/tiles` and `fog/bound` read 0.002 and 0.001 ms; their zones may overlap other
  work.
  - At 1280 × 720, DLSS Quality: the bar's clip 7.52 → 7.18 ms, the street's 7.72 → 7.51 ms.
  - The district's path at 2560 × 1440: `fog/light`'s zone 0.49 → 0.37 ms. The frame's gain is
    inside the runs' spread there: the lamps' ReSTIR alone moved by 0.26 ms between runs. Most
    of the path is outdoors, its far views reaching the volume's end.

**The history kept at silhouettes (2026-11-07, the owner's catch: "when the camera was moving, there was some kind of
trail behind the bodies").**
- **Found by clips, not stills:** the bar path at the same frames, as it was, without the history and without the cull. The
  owner picked the one without the history as free of the trail.
- **The cause:** a history read takes the eight froxels round the reprojected point. It was refused whenever one of them
  was culled. Along a body's edge, the froxels behind the body are culled, so the froxels beside them lost their history as
  the camera moved. They were then lit afresh at their unjittered middles, which stood apart from their neighbours' averaged
  light for the ten frames the history took to fill: a band trailing each body.
- **Now:**
  - the read takes the eight froxels one by one, leaves out the culled ones, and weighs the rest. It is refused only when
    none of the eight is lit.
  - a froxel without a history in a frame that has one is lit at its jittered point, like its neighbours.
  - with no history at all (the first frame, a cut, the history off), a froxel is lit at its middle, as before.
- **What it changes:** only motion. A still camera's frames ten apart differ alike: the bar 2.07 % of its pixels by over two
  codes with and without, the landing 2.09 %. That is more than this amendment's 0.71 % of 2026-11-05, because the signs
  now blink (D-104).
- **Cost:** none measurable. Alternated runs on the path gave a frame of 18.26 and 18.02 ms before, 18.26 and 18.07 ms
  with it; `fog/light` 0.46 and 0.45 ms before, 0.45 and 0.47 ms with it.

## D-100 — A scene's own colour grading: a 3-D LUT after a log encoding, as a display transform ✅ (proposed 2026-11-05; the owner's go the same day: "Follow recommendations"; D-045, D-083, D-084, D-094)

**The need.** The private district's look is graded by its source.
- **How the source grades:** its environment's `ColorGradingAreaSettings` encodes the exposed scene
  colour in ARRI's LogC, reads a 32 × 32 × 32 LUT (RGBA32F) and shows the result linearly (its
  SDR `TonemappingModeLinear`; `forceHdrLut` takes the HDR table).
- **What Forge does:** it tone maps with one of its curves (AgX's punchy look a stop brighter by
  default, D-045; ACES as Falcor's for the Bistro, D-084).
- **The curves differ along the grey axis** (display-linear output):

  | Exposed scene value | Source's SDR table | Source's HDR table | Forge's AgX punchy (+1 stop) |
  |---|---|---|---|
  | 0.18 (mid grey) | 0.25 | 0.33 | 0.21 |
  | 0.7 | 0.78 | 1.05 (clips) | 0.55 |
  | 1.4 | 0.94 | 2.06 (clips) | 0.72 |

  The source's curve is steeper through the mid-tones and reaches white about two stops sooner.
- **What the matched frames show:** the district's alleys stand +1 EV against the source's frames.
  Their darkest tenth matches, but their median is 3× and their brightest tenth 2.4× the
  source's. The exposure fits (D-083) can move the level, not the curve's shape.
- **The source's tables tried offline** (2026-11-05):
  - **Method:** four views of the benchmark path were captured as scene light (`--capture-linear`).
    Each was graded through LogC and the source's tables, with one exposure scale per view fitted
    so its median luminance meets the source frame's.
  - **Result:** the spread of tones then falls close to the source's, where Forge's curve
    doubles to triples it. The ratio of the brightest tenth to the darkest:

    | View | Source | Forge, AgX punchy | Through the source's SDR table | Through its HDR table |
    |---|---|---|---|---|
    | Alley | 51 | 108 | 37 | 9 |
    | Second alley | 83 | 142 | 41 | 9 |
    | Street | 83 | 272 | 118 | 68 |
    | Bar | 10 | 203 | 28 | 7 |

    The darkest tenth lands within about 1.5× of the source's (0.0037 against 0.0024 in the alley).
  - **Reading:** the look's shape is its grading, not its light.

**Proposed:**
1. **A display transform that reads a scene's LUT** (`Tonemap::SceneLut`, `r.tonemap lut` when a
   scene gives one):
   - the pre-exposed colour encoded by a named log curve (ARRI LogC v3 EI 800 first; linear as
     an option);
   - looked up trilinearly in a 3-D table (32³ or 33³, RGBA16F) bound bindless;
   - written display-linear, then sRGB-encoded as now.

   It is chosen at run time like the others, never baked into lighting, with a CPU mirror and its
   tests (`display.rs`). The HDR presets keep ACES 2.0 (D-070's HDR path): a scene LUT is SDR only
   at first.
2. **`Look::grading`:** a scene supplies its table and encoding. Forge ships no table of its own:
   the village and the Bistro keep their curves, and nothing changes for them.
3. **The private district** (its viewer, never in git) hands over its source's table. The exposure
   offset (`GAME_EV100`) is then refitted on the 14 matched frames: the aim is the percentiles'
   shape, not only their mean.
4. **Measured:**
   - the 14 frames' percentiles (p10/p50/p90) against the source's;
   - the pass's cost (one 3-D sample a pixel, inside the existing display pass);
   - 0 px in the village and the Bistro.

**Patents** (searched 2026-11-05, web research, not legal advice; Google Patents answered five
queries, then refused; Justia and FreePatentsOnline refused; Chinese, Japanese and Korean filings
not covered):
- **Low risk.** The proposal is Unity's Post-processing v2 "External" grading (beta July 2017,
  open source): `LinearToLogC` then a 3-D LUT. It is also GPU Gems 2's chapter 24 (Selan, 2005): a
  3-D texture with trilinear filtering behind a shaper.
- **More prior art:** ARRI's LogC usage paper (2011, the formula published, no ARRI patent
  found), OpenColorIO (shaper LUTs, 1.0 in 2011), Unreal Engine's grading LUTs, Frostbite's HDR
  grading (GDC 2017), O3DE's look modification.
- **Live patents read, none reading on it:**
  - ATI/AMD's reduced-LUT polynomial interpolation (US 11,115,563 B2), its LUT addressing by the
    float's exponent and mantissa (US 10,242,647 B2, US 10,867,580 B2), and its 2^m + 1 + 4p grids
    (US 10,424,269 B2);
  - the BBC's display-to-display conversions (US 10,455,209 B2);
  - Microsoft's histogram-driven tone curves (US 10,176,561 B2);
  - Dolby's LUTs blended by display peak (US 10,332,481 B2);
  - Sony Interactive's grading on a second machine (US 12,039,708 B2);
  - Apple's display LUT circuits.
- **The rules it keeps:**
  1. One fixed, published per-channel curve (LogC v3), then the hardware's trilinear sampling of
     a full, uniform 32³ or 33³ table: no custom addressing, no per-cube correction, never a
     2^m + 1 + 4p grid.
  2. The table authored offline and supplied by the scene: never derived or reshaped from the
     frame's content or histogram, the exposure a stage of its own.
  3. Before HDR tables blended by display peak or a rendering-intent conversion, search again.
  4. Before a live grading tool across two machines, read Sony's US 12,039,708 B2.
  5. LogC named only to describe the curve (ARRI licenses the name for cameras).

**Grading by area (2026-11-05, found after the proposal):**
- **What the source does:** besides its scene table, it grades by volume. A trigger volume over
  the private district's bar carries its own grading settings: per-channel gain (1, 1.18, 1) and
  gamma (1, 0.87, 0.95) over low and high tonal ranges. A second volume fixes the camera's
  exposure there (no auto-exposure). They cover all six of the bar's matched frames and no other.
- **What it explains:** applied roughly (gain then gamma on every pixel's display value), it
  brings two of the bar's frames' red-to-green ratio onto the source's (2.82 against 2.85, 2.04
  against 1.94), from about twice it. Three others overshoot to green, since the tonal ranges
  weigh it.
- **So a go would mean volumes too:** settings blended by the camera's place, as engines' post-process
  volumes do (Unreal's since 2014, Unity's Post-processing v2 since 2017).
- **Patents (searched 2026-11-05, Google Patents with CPC A63F13/52 and some 15 worded searches):**
  - **Cygames US 12,102,914 B2** (in force, about 2040; JP6987807B2): the closest. Its published
    claim 1 corrects the camera's image when the player's object enters a correction range set
    for another object whose movement the game controls. Its dependents add priorities among
    overlapping ranges and a blend range scaled by distance. World-anchored volumes lack the moved
    other object, so they do not read on it. **The granted claims could not be read** (the
    sites refused; snippets say they add the blend range and a saturation change).
  - **NetEase CN112206519B** (China, about 2040): trigger volumes a character enters, with
    environment parameters read from a file. Check it before a release in China.
  - **Capcom JP6457593B1** (Japan): needs a VR headset's three cameras. It is prior art against
    the above (per-area exposure, colour correction, fog, tone mapping, published January 2019).
  - **Sony US 2025/0299413 A1:** pending, a shader blended in near water; its claims not read.
  - **Prior art:** Source's colour correction by distance to an entity (c_colorcorrection.cpp,
    2005) and its volumes faded over time; Unreal Engine 3's post-process volumes (2007:
    priority, timed blends); Unreal Engine 4's blend radius, weight and priority (2014); Unity's
    Post-processing v2 volumes (2017).
  - **Rules:**
    1. Volumes anchored to the world (authored with the level), never attached to a character
       or an object the game moves: no grading aura round an enemy. Read the granted claims of
       US 12,102,914 before ever building that.
    2. Weights from the camera's depth into a blend margin, or a fixed time on entry; overlaps
       by priority, then weight.
    3. Before a release in China, check CN112206519B; once Sony's application is granted, read
       its claims.

**The owner's go, as read (2026-11-05, "Follow recommendations"):** the proposal as written; LogC
only for now; SDR only; the district judged with its source's grading (D-083's aim). Grading
volumes are a second step, under the rules above.

**Built (2026-11-05):**
- **The curve:** `Tonemap::SceneLut` (`r.tonemap lut`, `--tonemap lut`; not in the cycle), shader
  index 7. A scene gives its table through `Look::grading` (a `&'static Grading`: `size`³
  display-linear colours, red fastest, and its `GradingEncoding`, LogC v3 alone so far), which
  `Display::set_grading` uploads once. Without a table the curve shows AgX punchy.
- **The table's layout:** a 2-D strip of `size` slices, as ACES 2.0's table is (`aces2_lut`):
  the hardware's bilinear filter within two slices, the shader's blend between them. A 3-D image
  would have needed a new kind of bindless binding. The strip is the long-published layout of
  real-time grading tables (Unreal's 256 × 16 strips, Unity's `Lut2D`): a full, uniform grid
  read at fixed places, no custom addressing (rule 1 kept).
- **The CPU mirror:** `Grading::apply`, its tests (LogC v3's 18 % at 0.391, an identity table, the
  sizes). The start-up tone check (`meshlets --tone-check`) runs a fixed 17³ table on the GPU against
  it: 0.164 of an 8-bit code at most.
- **The push block:** `ToneTablesPush` 32 → 40 bytes, `DisplayPush` 112 → 120 (Vulkan's guaranteed
  128 asserted at compile time).
- **Elsewhere nothing changes:** the village's thirteen map-check views are 0 px from the build
  before.
- **The cost:** on the private district's benchmark path (DLSS Quality at 2560 × 1440),
  `post/display transform` 0.039 / 0.037 ms with the table against 0.044 / 0.040 with AgX punchy.
- **The private district through its source's table** (its viewer hands it over; the exposure's
  offset refitted to −1.28 EV, the offset on its range and its compensation alike, since the
  clamped frames follow the range and the metered ones the compensation):

  | 14 matched frames | AgX punchy | The source's table |
  |---|---|---|
  | geometric mean EV, rms | +0.19, 0.83 | +0.02, 0.64 |
  | median EV, rms | +0.63, 1.26 | −0.05, 0.71 |
  | p90 / p10 against the source's, log2 mean and rms | +1.21, 1.80 | +0.11, 0.78 |

  The bar's colours (red over green, blue over green, the source's first): frame 18 2.86 / 0.33
  against 5.19 / 1.12 with AgX and 2.20 / 0.31 through the table; frame 19 1.94 / 0.47 against
  3.72 / 1.22 and 2.09 / 0.51; frame 21 1.25 / 0.44 against 0.71 / 0.35 and 1.09 / 0.46.

**The grading volumes, measured before building (2026-11-05):** not built.
- **The method:** the bar's six matched frames rendered under the scene table, with and without the
  source's bar settings. The settings were applied in five forms, gain then gamma each time:
  1. on the log-encoded value, the gamma as a power;
  2. the same, the gamma inverted;
  3. on the linear scene, pivoted at mid grey;
  4. on the displayed sRGB value;
  5. on the log value pivoted at the source's 0.435, both ways.

  The measure: the log₂ RMS of the frames' R/G and B/G against the source's.
- **Colour:** without the area, R/G 0.35 and B/G 0.40. The closest form (2) gave R/G 0.30 but B/G
  0.67, 0.46 EV darker; the rest did worse on both.
- **Why:** the district's light was fitted on these same frames: the lamps' coloured lumens, and
  the fog's lamp phase checked on frame 21. Their colour already holds the area's grading, and
  applying it again counts it twice.
- **Exposure:** a fixed camera exposure (`--fixed-exposure`) leaves the bar frames' spread against
  the source's where it was: 0.62 EV standard deviation, metered or fixed. The metering's clamp
  already holds the bar's exposure steady. What spreads them is what is lit differently (frame
  14 +0.7, the landing −1.1).
- **So:** the volumes wait for a scene whose look they would change: a place graded by area whose
  lights were not fitted on its graded frames. The rules above stand for then.

## D-101 — The open city's distance: a hazier sky and a distance fog beyond the froxels ✅ (proposed 2026-11-05; the owner's go the same day: "Go for D-101, follow your recommendation"; D-032, D-083, D-094, D-099)

**The need.** The private district's open streets (its source's benchmark frames 36, 37, 41 and 44)
show a paler sky, a whitish distance (the far city fading into haze) and, in frame 41, a white glow
round the sun. Forge draws a deep-blue clear sky and a crisp far city. Beyond the froxel volume's
64 m (D-032, D-099), only the physical air hazes what lies behind.
- **The source's model:** its master environment's distance fog.
  - It starts at 200 m and ends at its far distance (1.5–5.5 km over the day).
  - It has two density layers, falling off with height (0.5 and 1), and colours near and far by
    the hour.
  - A simple fog alongside: one colour and 0.00025 a metre.
  - The sky is fogged up to an altitude.
- **A test (2026-11-05):** the physical air in the aerial perspective, made 20 times denser.
  - Frame 41 went from −0.72 to −0.43 EV and frame 44 from +0.10 to +0.38; the 14 frames' rms
    from 0.57 to 0.56.
  - The distance turned blue (the air's Rayleigh share is blue) where the source's is white. So
    the gap is aerosols and an authored fog, not more air.
- **A second test (2026-11-05 evening): the aerosols** (Earth's Mie scattering and absorption)
  times 3 and 6, everything else as it stands.
  - The 14 frames' rms 0.56 → 0.54 and 0.53. Frame 41 −0.70 → −0.58 and −0.48, frame 37 −0.12 →
    −0.02; frame 44 +0.12 → +0.22.
  - At ×6 the sky pales towards the horizon and the sun gets a white glow, as the source's frame
    41 shows it. The sun's share and the meter were not refitted for it.

**Options.**
1. **Aerosols only:** a scene's atmosphere with more aerosol (Mie) scattering.
   - The sky pales, the sun gets its glow and the distance whitens, with one setting.
   - The sky's light changes with it (the probes', the shade's), so the sun's share and the
     meter's compensation are fitted again after.
2. **A distance fog beyond the froxels:** an analytic exponential height fog.
   - Two layers, a start distance and a colour, lit by the sky's light and the sun's through a
     phase, as Unreal's exponential height fog is (Flax's is open source).
   - Applied in `fog/apply` to the pixels beyond the froxel volume, and on the sky up to an
     altitude. A few instructions a pixel in a pass that exists.
3. **Both (recommended):** the aerosols for the sky and the sun's glow, the fog for the
   authored distance. In the district, the viewer reads the source's curves for the scene.

**Patents** (searched 2026-11-05): none found for exponential height fog or distance fog.
- **Prior art:** Legakis's layered fog (SIGGRAPH 1998 sketch); Unreal Engine 3's exponential height
  fog (its UDK documentation); Quilez's closed form for height fog (2010).
- **The aerosols:** the atmosphere model's own (Bruneton and Neyret 2008, Hillaire 2020).

**Done when:**
- The open streets' frames match the source's in the sky's colour, the far city's contrast and
  frame 41's light near the sun.
- The 14 frames' EV and colour rms are no worse once the sun's share and the meter are refitted.

**Recommendation:** option 3, built in steps. Aerosols first (one setting, then its refits), then
the distance fog.

**Built (2026-11-05, both steps):**
- **Step 1, the aerosols:** `Look::aerosols` (city-blocks) multiplies the atmosphere's Mie
  scattering and absorption (`AtmosphereParams::mie_scattering`, `mie_absorption`) for the scene's
  sky, its aerial perspective and its sun's transmittance; the air's Rayleigh share stays. None:
  Earth's clear day, so every other scene is drawn as before.
- **Step 2, the distance fog:** `DistanceFog` (forge-render, `FogParams::distance`, a scene's
  `FogEnvironment::distance`), laid by `fog/apply` before the froxels' fog: no pass of its own.
  - **The medium:** two exponential height layers, each an extinction at a height and a falloff
    above it, integrated in closed form from the start distance to the pixel's surface (the sky's
    to 1,000 km). Each end's exponent is held within −80 and 20, and a ray crossing less than a
    thousandth of an e-fold is taken as level. `DistanceFog::optical_depth` is its CPU mirror; a
    test checks it against a numeric integral up, level, down, over a thin ground layer seen from
    200 m and along near-level rays to the sky.
  - **The start:** never nearer than the froxel volume's far end (64 m), which holds the near fog.
  - **The light:** the sun's through a Henyey–Greenstein phase, above the horizon, and the sky's
    every way alike (its irradiance up and down over 2π, as the froxels' without probes),
    unshadowed. Its colour runs from a near albedo at its start to a far one at its far distance.
  - **The sky:** its cover fades from the horizon to an elevation (`sky_end`); 0 leaves the
    layers' own integral.
  - **A distance fog alone** (no boxes, no fog everywhere) runs the froxels' passes over an empty
    volume.
- **The cost:** `fog/apply` about +0.01 ms on the private district's path at 2560 × 1440 (DLSS
  Quality); the frame within the runs' spread (`docs/PROFILE.md`).
- **The private district** (its viewer):
  - **Aerosols ×24,** fitted on frame 41's sky, the pixels both frames show as sky on the sun's
    side:

    | aerosols | sky EV | R/G (source 0.71–0.77) | B/G (source 1.13–1.19) | 14 frames' EV rms |
    |---|---|---|---|---|
    | ×1 | −1.41 | 0.28 | 1.80 | 0.56 |
    | ×6 | −0.95 | 0.48 | 1.57 | 0.53 |
    | ×12 | −0.57 | 0.67 | 1.37 | 0.52 |
    | ×24 | −0.09 | 0.84 | 1.18 | 0.51 |
    | ×40 | +0.39 | 0.94 | 1.07 | 0.53 |

    The source's sky has a turbidity of 3 by day, about ×40 by Preetham's definition at 550 nm.
    At ×24 frame 41 gets its white glow round the sun, and the open streets' colour error (the
    log₂ rms of R/G and B/G over frames 36, 37, 41 and 44) goes from 0.233 and 0.304 to 0.132 and
    0.233.
  - **The distance fog** reads the source's distant fog at the hour: near 200 m, far 3.35 km at
    16:20, densities 1.24 and 0.05 taken as the layers' optical depths from near to far at sea
    level, falloffs 0.5 and 1 per kilometre, colours near and far, the sky to 8.35°.
    - **Why per km:** per 100 m, a building 300 m away took 4 % fog and the far towers' tops stayed
      clear. The source's frame 41 shows them pale from top to bottom.
    - **The numbers:** few pixels lie that far, so the 14 frames' numbers barely move (frame 41
      +0.04 → +0.17 EV, the colour rms unchanged). The far ends of streets 36 and 37 and the towers
      in 41 now fade into a pale haze.
  - **The refits:** the whiter sky lit the open streets' shade, and at the old sun share (1/22 of
    Forge's) they read +0.06, +0.23, +0.17 and +0.62 EV (mean +0.27).

    | sun share | 14 frames' EV mean, rms | R/G, B/G log₂ rms |
    |---|---|---|
    | before D-101 (1/22, ×1, no distance fog) | −0.06, 0.56 | 0.262, 0.366 |
    | 1/22 | +0.06, 0.52 | 0.240, 0.367 |
    | **1/24 (kept)** | **+0.02, 0.51** | **0.243, 0.371** |
    | 1/26 | −0.02, 0.50 | 0.247, 0.377 |
    | 1/28 | −0.05, 0.50 | 0.249, 0.380 |

    - **Why 1/24:** a dimmer sun centres the open streets and moves their colour further from the
      source's at each step, so 1/24 sits between. Streets 36, 37 and 41 read −0.07, +0.09 and
      +0.03; frame 44 +0.47.
    - **Against this morning:** the EV rms and R/G are better. B/G is 0.005 worse, the one number
      that rose.
    - **The meter:** its frames moved −0.03 EV on average, inside its flat optimum, so its −0.5 stays.
- **Elsewhere nothing changes:** the village's thirteen map-check views are 0 px from this
  morning's run. No scene in the repository sets aerosols or a distance fog.

- **The sky's light on surfaces apart from the sky shown** (2026-11-06, `SkyParams::light_share`,
  `Look::sky_light`, 1 where none): the private district's source gives its sky far more light
  against its sun than the physical sky has (its tables at the hour: sun 0.66, sky light 1.30).
  - **Why it is the input's and not the GI's:** a true path trace of the same scene (48 rays, 1 to
    4 bounces) reads as the real-time GI does, a shaded pillar 17 against 18 where the source shows
    44, so the probes and the bounce are right about the light they are given.
  - **What takes it:** the sky's irradiance holds it from where it is projected (`irradiance_main`),
    with the factor stored beside it (`sky_light_share`), so surfaces, probes, impostors, panes,
    water and the fog in the air all take it (particles no longer do, below); the diffuse rays that meet nothing multiply
    theirs (`sky_light_along`, the bounce's, the probes', the caches'). The sky as drawn, as mirrors
    show it (`sky_along`) and in the clouds and the distance fog stays as it is.
  - **The private district, at 2:** the open streets' shaded half reads +0.10, −0.01, +0.02, +0.24
    EV against the source's frames (−0.27, −0.38, −0.41, −0.25 at 1). The 14 frames: geometric mean
    +0.10, rms 0.46 (0.46 at 1), the mean's rms 0.33 (0.35), the median's 0.59 (0.51): the sky's own
    pixels sit as they were, and the far towers above them are lit brighter.
  - **Left:** a reflection's sky share (`toward / open`, `sun_light`, `hit_metal_light`) weighs what
    isn't sky by 1 / the factor: a heuristic's small error at 2.
  - **Elsewhere:** 1 multiplies and divides exactly; the village's thirteen views score what they
    scored before.

- **The colour of the sky's light on surfaces** (2026-11-06, `SkyParams::light_white`,
  `Look::sky_white`, 0 where none): the same tables give the source's sky light a white colour
  (1, 1, 1, times 1.30) where the air's own sky is blue.
  - **What showed it:** with the sky's aerosols down to the source's table turbidity, the sky itself
    matched and the shade turned blue. The private district's alleys read R/G about 0.6 and B/G 0.34
    to 1.22 in shade, where the source shows 1.06 to 1.29 and 0.05 to 0.52.
  - **What takes it:** the sky's irradiance and the diffuse rays that meet nothing go that far towards
    their own luminance, which they keep (`sky_light_whiter`). The sky as drawn and as mirrors show it
    stays blue. The clouds and the distance fog take the sky's own colour back
    (`sky_light_as_it_is`), which is why it stops short of white ([`MOST_WHITE`], 0.95).
  - **Particles and spray without the share:** steam took the scene's fit to its walls as a lamp from
    above, and read bright blue-white in shade. A puff now takes the sky's light without the share, its
    whiteness kept. Its probes' light can't be told apart, so what lamps and the sun put in them falls
    with it, a known error: a puff under a street lamp takes that light at the same share.
  - **Left:** a rough reflection (`sky_reflection`) and the water's blur blend towards the whitened
    irradiance, so a rough patch reads greyer and brighter than a mirror beside it. The former is the
    light on the surface, as its diffuse takes it; no scene with a share has water.
  - **The private district, at 0.9 with a share of 3:** the 14 frames' median rms 0.62 to 0.49 EV,
    geometric mean rms 0.47 to 0.42, frame 44's colour R/G 1.03 and B/G 0.97 to 1.09 and 0.91 (the
    source's 1.10 and 0.90). The alleys' shade reads R/G 0.96 to 1.04, B/G 0.11 to 0.63.
  - **Open:** the source's shade may be warm for another reason as well, a bounce we under-light, since
    a white sky is a fit to its tables' colour and not a measure of its light.
  - **Elsewhere:** 0 and a share of 1 are exact: the sky's light, the clouds, the fog and the steam
    read as before where a scene sets neither.

## D-102 — A lamp's channels: an authored scene's lamps light the objects of their zone ✅ (the owner's go 2026-11-06: "go for the per-object lighting zones"; D-065, D-083)

**The need.** A shipped open-world city game groups its interior lamps by zone. A static light carries a
set of channels, and volumes give those channels to what stands inside them, so a lamp over a bar's lower
floor lights that floor's objects and not the gallery's. Forge lit every surface in reach.

**What came first.** A lamp's light held in a box round its zones (position, not object) changed nothing
in the 14 matched frames, nor did each lamp's hide distance. Both were removed and never pushed. Membership
by position can't tell a wall whose middle lies outside a zone from one inside it.

**Decided.** Two masks, met at the surface:

- `Lamp::channels` (`GpuLamp::flags.z`) and `MeshletSceneBuilder::set_instance_channels` (`Instance::channels`,
  the record's spare word). A surface takes a lamp where the masks meet. Zero on either side means every one,
  so a scene that sets neither reads exactly as before.
- **Where:** `lamp_takes`, in `lamp_weight_in` and `hit_lamp_weight` (the lists, the far lamps and the rays'
  hits), and in `restir_lights`. ReSTIR keeps each pixel's mask beside its place for its neighbours (8 bytes a
  pixel over the two frames).
- **What it leaves alone:** the fog and the particles take every lamp, and the sun and sky are not channelled.
  A grass blade, a mover, an object a `SceneEditor` streams in (it has no setter yet) and a baked hit take every lamp. A mask of the lamps' is a scene's data: the engine gives
  the bits no meaning.

**Which objects count as inside** is the scene's call and isn't known for the source. In the private
viewer an object's pivot reads best. Its bounding-box middle left one frame's floor at 0.004 against the source's
0.055 (pivot 0.024), since a large floor's middle can fall outside its zone.

**What it did.** A small gain on the private district's 14 frames, not the large one hoped for: mean rms 0.35 to
0.34, geometric 0.44 to 0.43, median the same. One frame's floor went from -0.24 to -0.03 EV, with its red closer to
the source's (R/G 18.8 to 21.3, source 22.8); another from +0.16 to +0.07; a third from -0.19 to -0.30. Every lamp
there casts shadows, so a zone's walls already hold most of its light in. The over-lit wall that prompted this stayed
over-lit: the object's pivot is in the lamp's zone. The hunt for it found the next item instead.

**Found on the way: a room's rough surfaces no longer reflect the sky.** The sky's reflection scales the sky's picture
by the probes' light towards the mirror direction over the open sky's (#68). That ratio measures light, not sky, so in
a room a lamp's bounce or a door's daylight brought back the sky's picture in its own colour: a red room's floor read
blue-white with every lamp off. `room_sky` allows all of the sky outside, none in a room, and between the two across a
doorway. A smooth surface's mirror ray still sees a real window.

**Patents.** A light mask per object is old practice: light linking in offline renderers, lighting channels in
released game engines. Two searches found no live patent on it. The closest, US 6,639,595 (a per-channel light
mask in fixed-function hardware), was filed in 2000 and has expired. The searches were not exhaustive.

## D-103 — Pictures of light with no surface: a pass over the HDR scene ✅ (the owner's go 2026-11-07: "go with people, bottles and holograms"; D-065, D-083)

**The need.** The private district's shipped open-world city game draws floating signs, prices over stalls and pictures on fridge doors as light with no surface: a texture added over what is behind it. Forge's visibility pass has no such thing. Drawn solid, they stood as dark slabs, so the viewer left them out: 42 pictures on 5,831 triangles along the benchmark's path.

**Decided.** `forge_render::Holograms` and `shaders/holograms.slang`, `Running::holograms`: triangles in the world frame, each corner naming a row (a picture, its glow in cd/m², its hue, saturation and value). One pass, `holograms/draw`, after the glass: the picture's colour times its alpha, times the glow and the exposure, added to the HDR scene where nothing nearer stands. It tests the frame's depth itself and writes none. A scene that hands over none has no pass and no buffers, so every other scene renders as before.

**What it leaves out.**

- No surface means no light taken, no shadow, nothing in the ray-traced scene and nothing lit by it. A picture's glow on what is near it isn't drawn.
- It writes no depth, so the upscaler reprojects a pixel by what is behind it. A picture in front of something near may trail a little when the camera moves fast. Not seen yet; to look at on a walk.
- The glass writes no depth either, so "where nothing nearer stands" doesn't count a pane: a picture behind one isn't dimmed, and shines through a roller blind.
- The sky's aerial haze isn't laid over a picture, as it is over the glass and the particles, so a far one keeps its full light. The froxels' fog, drawn later, still reaches it.
- The picture's level of detail is the plain derivative's: the upscaler's texture bias doesn't reach it, so a picture is slightly blurrier than the scene under DLSS Quality.
- The source's pictures carry up to four layers parted along the view, which is what makes them read as deep, plus scan lines and scrolling. Only the first layer is drawn: **Sony US 12,293,452 (a live patent on layered atlases, D-060) is to be reread before any layering**, so none is built here. The pass has no flipbook: a scene that wants one tile gives its corners that tile's places.

**Patents.** Adding a texture's light over a scene in a blended pass is old practice. No search was run for it specifically, because the pass has no claimed trick in it; the one live patent known near this area is the layering above, which it avoids.

**What it did.** `holograms/draw` 0.009 ms on the benchmark's path at 2560 x 1440. The pictures are small on screen, so the 33 scored frames barely move.

**Amended (2026-11-07): rows that move** (D-104). A row has `HologramRow::uv`, a transform of the corners' places in the picture, and `HologramRow::motion`: a flipbook or a scroll over that transform, and a glow that blinks or flickers.
- **How:** the moving rows are worked out at `HologramParams::time` and written into the frame slot's own rows. The shader reads the transform and its implicit gradients follow it.
- **In the private district:** the pictures' flipbooks play at the source's rate, for example 5 by 20 frames at 10 a second.
- **Still one layer:** no layers parted along the view, so the patent note above stands.

**Also landed with it:** `smoke-cigarette`, a particle effect in `data/particles.toml`, a thin thread of smoke for a held cigarette, lit by the room's lamps. The private viewer emits one at each smoker's hand. Its values are a fit by eye: the source's own effect wasn't read.

## D-104 — Materials that move with the scene's clock: their rows rewritten per frame slot ✅ (#398; the owner, 2026-11-01: "There are plenty of animated things like screen or other supports, you might need to support that as well"; 2026-11-07: "follow recommendations and the plan")

**The need.** Forge's materials stood still: a screen showed one picture, and a sign's glow never blinked. The private district's source animates thousands of them. Among its mesh materials, 2,655 signs blink, 422 neon tubes flicker, about 440 screens play flipbooks of frames, and some 20 surfaces pan their texture coordinates. Comparisons with its frames kept meeting this: a board showing another advert than the source's frame is only timing.

**Decided.** `MeshletSceneBuilder::set_material_motion(row, MaterialMotion)` and `MeshletScene::set_material_clock(seconds)`, in `forge_render::material::motion`:

- **Four motions:**
  - a scroll, in texture coordinates a second;
  - a flipbook: a grid of frames (columns by rows, the first `frames` of them) played at a rate;
  - a glow that blinks: period, duty, phase, low level, eased edges;
  - a glow that flickers: steps a second, a chance to dip, the lowest dip, a seed.
- **Where it's worked out:**
  - On the CPU, once a frame, in f64 at the application's clock.
  - The moving rows are written into the frame slot's own copy of the material table, and the frame block points at that copy. Every pass of the frame reads the same rows, so the ray hits and the probes see the same glow as the screen does.
  - The copies are in mapped device memory (Resizable BAR), as per-frame data is. A device without it reads them over the bus, with a warning.
  - The table as built serves as frame slot 0's, so the cost is one more copy per other slot (one, with two frames in flight). The structures' builds keep reading the table as built: a moving row is never cut out, and its flags and alpha, the fields they read, never change.
- **What moves in the row:**
  - Its UV transform (`uv_u`, `uv_v`) and its emissive colour. The shader already reads both, so no shader changed.
  - The texture gradients follow the transform, so a flipbook's cell takes its own mip level.
- **A scene without motions** keeps its single table, and the frame block points at it as before.

**What it leaves out.**

- **Rows that can't move their coordinates:** a row that is cut out (masked, jelly, a house's clear glass), layered, or a room atlas. Its opacity micromaps and the structures' opacity checks keep the row as built. A glow alone suits any row.
- **No blending in a flipbook:** it steps from frame to frame. Blending would need a second sample in the shader.
- **The upscaler:** the geometry doesn't move, so the motion vectors are zero and the upscaler takes a scrolling picture as changed shading. It may smear a fast scroll a little; #360's reactive mask is the hint for later.
- **Lamps:** a blinking lamp's glowing part blinks, but its light (the lamp lists, RTXDI) is the application's to switch.
- **The probes and the caches** take in a glow's change at their own pace, as they do any change of light.
- **Later steps of #398:** video on surfaces. The holograms' motions came the same day (D-103's amendment).
- **The source's formulas aren't known.** The private viewer maps the source's parameters onto these four motions by their names and values.

**Patents.** Animated textures by frame sequences and by scrolling coordinates are 1990s practice. Nothing here goes past it, so no search was run.

**Checked.**

- **Unit tests of each motion:**
  - a flipbook's order and wrap, and its cell on the row's own transform;
  - a scroll's wrap an hour in;
  - a blink's duty, phase and eased edges;
  - a flicker's share of dips and its repeat by seed;
  - which rows refuse moving coordinates.
- **A GPU test of the slots' tables:** the moving row is written at the clock, once a frame, and the other rows stay as built.
- **Scenes without motions are unchanged:** the guard captures `ast240` and `lab-creatures120` differ from the code before by 0 px.

**What it did** in the private district, on its benchmark path at 2560 x 1440 under DLSS Quality. Within the path's reach, 578 rows move: 355 blinks, 129 flickers and 94 flipbooks.

- **The frame:** no clear cost. With the rows moving, 17.79, 19.07 and 18.08 ms; still, 18.27 and 17.68 ms. The 19.07 ms run was slow in its probes, and its p99 was 28 ms.
- **VRAM:** the table holds about 800,000 rows, mostly layered materials' layers, so a copy is 193 MiB. Two copies over the table as built cost +386 MiB; since the table as built serves slot 0, the cost is +193 MiB.

**Amended (2026-11-07): slideshows** (the owner's go: "go with the ads slideshow"). `MeshletSceneBuilder::set_material_slides(row, Slideshow)` makes a row show pictures in turn.

- **A slide:** a sampled image and the transform of the row's coordinates onto it. Each shows for `seconds`, starting from `offset` into the show.
- **What a slide changes:** the row's colour and glow maps, where the row has them, and its transform. A motion of the same row applies over the picture.
- **Cost:** they ride in the same per-slot tables as the motions, so nothing more than the rows written.
- **Refused:** a show without pictures, one of no length, or a row that can't move its coordinates.
- **In the private district,** 299 advert boards cycle through every district's fallback picture of their format, 8 s each. That interval is a guess at the source's rotation. Each board starts from its own picture, and boards change apart from their neighbours. The source's boards play videos; their stills are what the slideshow cycles.

## D-105 — Textures streamed by whole levels, by need, into a pool ✅ (proposed 2026-11-07, #385; the owner: "move the effort to ... texture streaming (#385)"; accepted the same day: "ok for D-105")

**What was measured** (#385's first step, `FORGE_TEXTURE_FEEDBACK=1`, `meshlet/feedback.rs`). The private district on its benchmark path at 2560 x 1440:

- **Resident:** 16,832 textures, 4,805 MiB.
- **Needed over the path:** 1,498 MiB (DLAA 1,469).
- **Within any 2 seconds:** mean 830 MiB, largest 908 (DLAA 840, 902). Of that, 601 MiB are the levels of 128 texels and less.
- **Never wanted along the path:** 80 % of the textures.
- **DLSS Quality wants what DLAA wants,** through D-070's mip bias.
- **The load:** it uploads 8.6 GiB on one thread in 24 s, the textures most of it.

**Proposed:**

1. **Whole levels per texture,** not pages. Levels already reach a fifth of the memory. Pages would also save the unseen part of a texture that is partly seen, but they cost a page table and an indirection in every sample.
2. **A pool sized from the measure** (1 GiB here), each texture resident from its wanted level down. The levels of 128 texels and less stay resident; 64 is to be measured.
3. **A change of levels makes the image again:** its new top level is loaded, the coarser levels are copied over, and its bindless slot is swapped between frames. Sampling uses whatever levels the image holds. No clamp table.
4. **One predictor:** what the frames sampled. No camera or distance predictor is merged into it.
5. **The files:** each level readable alone (KTX2 with per-level supercompression), held in the package (#214).
6. **What decides a texture's level:** the finest level its samples wanted within the window, from the feedback as measured.

**Accepted** (the owner, 2026-11-07: "ok for D-105"), steps 1 to 6.

**How it is built (2026-11-07):**
- **A remade image takes a new slot.** Material rows hold bindless slots, and Vulkan doesn't allow rewriting a
  descriptor that a frame in flight may still sample. So a remade image is registered in a new slot. Its old image and
  slot retire after the frames that may sample them.
- **The rows follow, frame slot by frame slot.** A row that refers to a streamed texture is rewritten into each frame
  slot's own material table with the texture's current slot, as D-104's moving rows are.
- **The material table's textures first.** A texture that decals or holograms also refer to stays resident until their
  tables follow too.
- **The steps:**
  1. Remaking an image, uploaded whole. Copying the coarser levels from the old image would take it out of its
     sampled layout while frames on either queue may read it (the probes' rays on the async queue sample materials
     too). Those coarser levels are a third of the new top's bytes. Tested.
  2. The rows' remap in the frame slots' tables.
  3. The policy: the feedback's levels, the pool, eviction by last use.
  4. The files: each level readable alone (KTX2) in the package.
- **Step 1 is built (2026-11-07):** `TextureSet::remake` and `remake_compressed` make a texture's image again from a
  level down, uploaded whole into a new slot. The old image and slot come back as a `RetiredTexture`; dropped through
  `Frames::destroy_later`, it frees both. A GPU test covers fewer levels and all of them back, RGBA8 and BC1, and the
  set's bytes.
- **For step 3:**
  - **The feedback's levels count from the image's current top.** A remade texture's wanted level in its full chain is
    the recorded one plus its top.
  - **Masks and presence maps stay resident.** The shading counts the presence map's blocks on the masks' full size
    (`multilayer_light`), so the streamed textures are the maps: colour, normal, roughness, metal and microblend.
- **Step 3's policy is built (the same day):** `texture_stream::LevelPlanner`, pure CPU, unit-tested.
  - **What it holds:** each streamed texture's levels from a top down. Its tail (128 texels and less) is always resident.
  - **Its target:** the finest level wanted within the window; once that want is older than the window, the level
    wanted last; else its tail.
  - **A plan:** down to the targets first, then up by the largest gap, each as far as the pool allows. It uploads at
    most the bytes it is given, but always makes at least one change.
  - **Eviction:** textures wanted strictly longer ago drop to their tails, the oldest first. Textures wanted in the same
    frame never evict each other.
  - **Not wired yet** to `remake` and the rows' remap (below).
- **Simulated on the private district** (`FORGE_TEXTURE_FEEDBACK=1`: the frames' wants fed to a planner over the
  block-compressed maps, nothing streamed). The benchmark path at 2560 x 1440 under DLSS Quality, 5,000 frames, at most
  32 MiB remade a frame:

  | | Today | With the planner |
  |---|---|---|
  | The 7,724 maps (colour, normal, roughness, metal, microblend) | 1,370 MiB resident | 167 to 246 MiB held |
  | What it uploads | | 1.2 GiB over the run, 13 to 16 MiB/s |
  | Wants whose levels were resident | | 99.7 to 99.8 % |
  | What stays resident beside it (masks, presence maps, other RGBA textures) | 3,842 MiB | 3,842 MiB |

  Pools of 1,024 and 512 MiB behave alike: the maps never fill them.
- **What this changes:** the maps save about 1.15 GiB. The layered masks are the larger part, 3.4 GiB of uncompressed
  RGBA8. They have two ways down, both the owner's call since each touches the look:
  - **Streamed like the maps:** the shading would count the presence map's blocks from the presence map's own size,
    not the masks' (`multilayer_light`).
  - **Compressed:** BC4 a mask halves them.
- **A first simulation over every texture, masks included, said a 1 GiB pool was enough. It was wrong:** D-105 keeps the
  masks resident.
- **What step 2 must hold (found while planning it):**
  - **Which fields are slots:** the colour, normal, metal-roughness, occlusion, emissive and presence maps are slots on
    every row. A microblend row (`MATERIAL_MICROBLEND`) keeps its microblend's slot in its first mask field, and
    floats (tiling, contrast) in the others. A blind "replace the old index" could hit a float whose bits match.
  - **The rays read slot 0's table for every frame** (`set_materials` at the build). So a rewritten row reaches the
    rays only when slot 0's table is written. The old image may retire only after every slot's table, slot 0's
    included, holds the new index, and the frames that read it are done. Retiring at the frame of the remake would
    free an image that a later frame's rays still sample.
  - **Slideshows' pictures, decals and holograms hold slots too.** Their textures stay resident until they follow.
- **Step 2's first part is built (the same day):** `GpuMaterial::texture_slots` and `remap_texture` know a row's slot
  fields by kind. A microblend row's floats are never taken for slots, even when their bits equal the slot moved; a unit
  test checks that case.
- **Step 2's scene part is built (the same day):**
  - **Opting in:** a scene built with `set_streamed_textures` keeps its table as built and a table per frame slot, as
    a scene with moving rows does.
  - **The remap:** `MeshletScene::remap_texture(from, to)` moves the slot in every row. Each frame slot's table takes
    the rows as its slot comes up, slot 0's included, which the rays read.
  - **Retiring:** `texture_remap_done(generation)` says when every slot holds the remap. From that frame on, the old
    image retires through `Frames::destroy_later`.
  - **Tested:** a GPU test. Scenes that don't opt in are unchanged.
- **A second review's catches, fixed the same day:**
  - **The feedback couldn't ask for more.** It measured the level against the bound image and clamped it at that
    image's level 0, so a texture remade at its tail could never grow. It now records the level from the bound image,
    negative when finer (`NEED_BIAS`), and `wanted_in_image` adds the top.
  - **An evicted texture could grow back in the same plan.** It no longer can.
  - **A slideshow's picture was remapped.** Its texture is written into its row every frame, so `remap_texture`
    refuses it and the texture stays resident.
  - **The first simulation streamed the masks too.** It is redone above over the maps alone.
- **Next:** wiring the planner, `remake` and the remap together in the private district (the maps only), then the
  files (step 4).

**Amended (2026-11-07): the masks in BC4** (the owner: "go with BC4 for the masks"). The layered materials' masks are
3.4 GiB of uncompressed RGBA8 on the private district. Stored as BC4, a mask takes half the bytes.

- **The measure that preceded the call:** a plain min/max BC4 encoder on a sample of 40 of the district's masks (3
  million texels).
  - 70 % of the blocks are flat and come through exactly.
  - The mean error is 0.22 codes and the largest 18.
  - 2.8 % of the texels move by more than 2 codes, 0.42 % by more than 8.
  - The ISPC encoder already in the engine (`intel_tex_2`) does at least as well.
- **How it is laid out:** two masks a texture, as BC5. BC5 is two BC4 channels, so each mask gets BC4's quality.
  - A setup has up to 19 masks on the district. One BC4 texture per mask would fit only 8 in a row's eight mask fields.
  - BC5 pairs fit 16 there, and the row's two spare words take the count to 20. The row stays 240 bytes.
- **What changes:**
  - `layer_mask_textures` encodes the pairs.
  - `multilayer_light` finds mask `m` in field `m / 2`, channel `m % 2`. The microblend's reading of its first mask
    field is unchanged.
  - The setups' stored form changes, in the derived cache and the package.
- **Checked before it lands:**
  - the district's baseline stills and scores (`tools/baseline.sh compare`);
  - the load: the encoding is made once and then stored as made;
  - VRAM.

**Built (2026-11-07, night):**
- **The code:**
  - `Multilayer::pairs`, with ten mask textures (the last two in the row's two spare words).
  - `MATERIAL_MASK_PAIRS`.
  - `layer_mask_textures_bc5` (`dds::masks_to_bc5` per pair).
  - `layer_mask_image` and `layer_mask_channel`, read by the shading and the feedback alike.
  - With `pairs`, the streamer's texture fields take the two extra slots too.
  - RGBA8 masks stay as they were, four to a texture, for a caller that keeps them.
- **The private district** (its viewer makes its setups' masks in pairs, stored so in the cache and the package,
  format 2):
  - **Memory:** the textures 5,099 → 3,267 MiB at the bar's crowd view (2560 × 1440, DLSS Quality); VRAM 85 → 75 %
    of the budget.
  - **The load:** the setups encoded once in 6.6 s (2,107 of them), then 1.3 s from the cache.
  - **GPU:** 18.8 → 19.4 ms. The ray passes that never read a mask moved as much (ReSTIR +0.27 ms, the traces
    +0.06 to 0.07 ms), so most of it is run drift. `shading/standard` +0.12 ms.
  - **Images against RGBA8:**
    - the crowd close-up: 0.027 % of the pixels by more than 4 codes (small marks on a door frame);
    - the bar's shelf view: 0.12 %.
  - **An older package's setups** (format 1) are made again from the mask files, with a warning: write it again.
- **Not done: the crowd's masks above 256 texels.** At 1024 as RGBA8 the textures took 2.09 GB more, VRAM 98.7 %,
  and the frame 18.8 → 79 ms by paging, for no face drawn sharper (the one seen closest wears a mask).
  - In BC5 it would still be about 1 GB.
  - That asks for levels by distance (this decision's streaming), not a global cap.

## D-106 — An authored world shown by the camera: each piece's own range, real pieces and stand-ins in one table ✅ (accepted by the owner 2026-11-08; being built, step by step)

**Why:** the private district draws a fixed set of pieces, chosen once for the whole run, and a free camera goes where that
choice is wrong. The far city's stand-ins took five rules in one night, each mending what the last one broke:

- real geometry missing with the stand-ins gone;
- stand-ins lying over real roads and buildings (z-fighting, bright blurred patches);
- whole terrain tiles either standing through the real ground or leaving a hole in it.

The last of them cannot be settled by any fixed choice: one threshold fixes one view and breaks another. Every piece is also
resident at once: about 13.4 GB of VRAM, 92 % of the budget.

**What the source's own data holds** (the hand-over measured on the 1,834 sectors first read, the rest on the full table of step 1):

- **Each piece has its own range:** a reference point and a streaming distance. The reference point is not its origin; one
  stand-in's origin stood 1.6 km from its geometry, which is what misled the rules above.
- **Stand-ins also have a near distance:** closer than that they are hidden.
- **Real pieces name their stand-in:** 1,597,131 placements carry a link, and 99.8 % of the links resolve to a stand-in in the data.
  Terrain chains: a tile names its stand-in, and that one a coarser one. An entity's stand-in names its entity instead, with the distance
  the entity attaches at.
- **Pieces hand over in groups:** 22,147 stand-ins have linked pieces (464,203 of them), and within a group every piece has the same
  streaming distance. That reach passes the stand-in's near distance by a median of 16 m (p10 6, p90 49), so both exist for a stretch.
  Forty groups stop short by more than 5 m (signs and billboards capped near 512 m, the worst by 2 km): there neither would show.
  The farthest of a group's reference points from its stand-in's is 0 m in the median group, 57 m at p90, 1.2 km at worst.
- **Some stand-ins have no linked piece:** 54,589 of 126,553, with every sector read: the roads', and the coarse hulls of whole
  neighbourhoods.
- **A camera needs little of it:** 29,107 to 70,787 placements are live at the cameras tried, out of 4.6 million, using 3,369 to 6,805
  of 32,378 distinct meshes.

**The rule, checked on the data at the owner's failing views:** a piece is live when the camera is within its streaming distance
of its reference point; a stand-in also needs the camera beyond its near distance.

| View | What went wrong | What the rule does |
|---|---|---|
| Street at the centre | three stand-ins over the road | all three hidden |
| Tall building, from the street | upper floors missing | their stand-ins shown |
| Beach and shore | terrain stand-ins through the ground, or a hole | nearest six each hidden; 28 and 16 real tiles live |

These are checks of the rule against the data, not renders.

**The design: the rule runs on the cell streamer's worker (D-096).**

- **Cells stay the unit of reading and of mesh lifetime** (`SharedMeshes`). A cell's reach comes from its own pieces, the largest
  streaming distance among them, in place of one reach per level (`StreamPolicy::rings`).
- **Inside the loaded cells, each piece is tested on its own** as the camera moves. Pieces enter and leave the scene through the
  same scene edits as a cell's instances.
- **Rays and raster agree without further work,** since both draw from the scene's instances.
- **A stand-in and its linked pieces swap as one group.** The group is live while any of its pieces is in range: all its pieces stay and
  the stand-in waits. Out of range, the stand-in shows. No group is drawn in part or twice, and the forty short groups have no gap.
- **A stand-in with no linked piece obeys its near distance,** as the source does.
- **One table:** real pieces and stand-ins come from the same converter. The separate far-city list and its rules go, with the
  district's fixed radius.

**Left out, and why:**

- **A distance test per instance in the GPU cull:** exact each frame, but the rays keep every piece, so a stand-in's shell would
  shadow the real building inside it. Changing what the rays see means a top-level rebuild anyway, which the edits already do.
- **Keeping a stand-in with no linked piece shown inside its near distance,** to avoid holes: the three over the road are of that kind,
  and 104 to 199 of them per camera would come back. The holes to fear are pieces not yet read, which step 1 reads and counts.
- **Cells alone, one reach per level:** ranges differ too much within a level (at the finest, 19 to 108 m from p10 to p90), and a whole cell cannot
  swap with stand-ins that do not follow its edges. That is another fixed approximation.

**Steps, each checked before the next** (the owner's six views, the tall building, `tools/baseline.sh compare`):

1. **The data** (private; no picture changes): the range, near distance and link per piece, in one table. Read the 1,993 sectors
   round the district not yet extracted. Then, before any engine work, count over a grid of cameras the stand-ins hidden by their
   near distance with no real piece over their footprint: the holes to expect.
2. **Pieces by their own range in the streamer,** with the district streamed. Measured: the views above, VRAM, the worker's time per
   edit, and the 99th percentile at a walk and at 40 m/s.
3. **The streamed path catches up with the resident one:** the crowd, the holograms, the rotating pieces and the lamp channels are
   missing or simplified there today. Then it becomes the default.
4. **Lamps and decals by their own ranges** (a lamp at the finest level hides at a median 30 m), measured for the frame's cost.
5. **A fade across the hand-over,** if the walk shows pops: the 16 m overlap is where it would run.

**Step 1, done (2026-11-08).** The private converter writes the range, near distance, link and owner per placement, stand-ins in the
same table: 3,825 of 3,827 sectors (two cannot be converted, one small cell each at the area's north edge), 4.6 million placements. Over a
grid of 529 cameras, a median of 146 stand-ins with no linked piece are hidden by their near distance (276 at most). Twenty of the 55,051
such stand-ins ever lack real geometry under half their footprint, all neighbourhood hulls or coarse terrain 269 m to 8 km wide that finer
stand-ins replace. What this table cannot judge, tall stand-ins whose top third the real pieces do not fill (83 per camera, mostly the
same hulls), waits for step 2's pictures.

**Step 2, measured (2026-11-08).** The ±1,100 m area streamed, pieces by their own ranges: 41,050 meshes, 4.6 million placements, 2,859
cells. Off by default, as planned until step 3.

What the first runs changed:

- **A mesh is held by the shown pieces using it,** not by the cells. Held by cells, 22,800 meshes could be in at the worst place for
  3,400 to 6,800 on screen, and the card could not hold them. The tables are reserved for the most that can show at a populated cell plus
  a quarter: 10,172 meshes and 34 million ray vertices, for 40,429 meshes and 166 million.
- **The first view arrives in 10 edits of 768 new meshes,** about 120 ms each on the worker, pieces going only once all those coming are
  in, so no stand-in leaves before what replaces it. A jump of over 200 m lets the old view go at once.
- **Setups with the same mask files share their textures:** 6.14 to 3.93 GiB of masks here, 2.20 to 1.48 in the resident district.
- **The bindless table holds 262,144 sampled images,** up from 65,536 (this area needs about 62,000 with the masks shared; the
  devices tested allow 1,048,576 and more). The resolve's two 16-bit indices became two words, the push constants still 128 bytes.
- **A chained stand-in's siblings come in together** (the batch's review): one tile in reach took its coarser parent away while a
  sibling past its own distance stayed out, leaving neither level over that patch.

The checks, at 2560 x 1440 under DLSS Quality:

| | Walk (benchmark path) | 40 m/s tour |
|---|---|---|
| GPU per frame | 18.4 ms (the resident district: 17.1) | 9.4 ms |
| Frame time p50 / p99 | 18.4 / 24.5 ms (resident: 17.1 / 19.2) | 9.0 / 21.0 ms |
| Video memory | 11,504 MiB | 12,018 MiB |
| Worker per edit p50 / p99 / most | 2.5 / 5.7 / 9.5 ms | 2.0 / 7.1 / 17.8 ms |
| New meshes per edit, mean / most | 0.6 / 36 | 2.3 / 89 |
| Meshes held, most | 6,998 | 6,955 |

**Those timings draw textures of 512 texels or more one level down,** 4,394 MiB for the full 7,180, so that they fit. At full textures the area
holds 13,520 to 14,096 MiB of the 14,600 the card gives it and the stills' frames take 18 to 69 ms: the first view goes over budget and the frames stay slow.
The pictures are right, the timings are not. D-105's wiring is what brings full detail in budget.

**The owner's walk (the same day): "LOD is too close, the pops are obvious."** How large pieces are on screen when they come or go, in pixels of
their bounding sphere's diameter at 1440p:

| What changes | Median | 90th percentile | Over 64 px |
|---|---|---|---|
| A stand-in, at its swap | 134 | 359 | 88 % |
| A group's member, at the swap | 36 | 94 | 20 % |
| A piece alone, at its range | 19 | 70 | 13 % |

Scaling every range, along the walk with textures a level down (another session shared the machine, so small differences mean little):

| Ranges | Meshes held | Reserved meshes | Video memory | Frame time p50 / p99 | Stand-in at its swap, median |
|---|---|---|---|---|---|
| × 1 | 7,038 | 10,172 | 11,789 MiB | 17.6 / 34.3 ms | 134 px |
| × 1.5 | 10,388 | 15,510 | 12,555 MiB | 19.1 / 39.5 ms | |
| × 2 | 13,767 | 20,093 | 13,937 MiB | 19.9 / 142 ms | 67 px |
| × 3 | | 27,070 | out of memory | | |

Longer ranges alone do not end the cuts: at twice the range a typical stand-in still changes across 67 pixels. So step 5's fade is due, and the
ranges want lengthening by what memory allows once D-105 frees it. At full textures × 1.5 already runs out of memory.

Open from these runs: from the five high views the streamed area's shadows are darker than the resident district's, 0.7 to 1.6 stops in the
shaded tenth of the picture, 0.2 or less at the two street-level views. The geometry is there; the cause is not yet known.

**Step 5, built (2026-11-08), waiting for the owner's walk.**

- **What:** a piece dissolves across a band of distance instead of cutting. A stand-in and its group's pieces share one crossing and the same
  noise, so each pixel keeps one or the other: no hole, no double. It is D-075's dithered ladder crossing with a distance in place of a size on
  screen, the frames' noise averaged by the anti-aliasing's history; no new technique to clear.
- **Measured each frame in the instance cull,** from the viewer to the crossing's point, so it follows the camera exactly. A table per instance
  slot (`SceneEditor::set_instance_fade`, 48 bytes, 14 MiB for 300,000 slots) holds each side's point, distance and band. A piece mid-fade takes the
  cut-out raster; the value is written where a ladder's crossing is, as how far it has come and how far it has gone.
- **`RangedSet` resolves each piece once into the crossing it comes at and, for a stand-in, the one it goes at.** A group shares one, round the middle
  of its members' references and as far as the farthest member reaches, which replaces the test of which members are in range: a coarser group's
  stand-ins then come together, and a coarser crossing is kept at least as far as its members' own.
- **The pieces are in before their band:** the worker brings a piece 8 m ahead of the band's outer edge, and takes a stand-in out 8 m past its inner
  edge, undrawn there, so the shader's crossing never runs ahead of the worker's edit (the batch's review: measured from the camera alone, the first
  pixels of a dissolve had nothing behind them).
- **The band:** 10 % of the swap distance, between 8 and 40 m, and never wider than the distance, so that a piece with a very short range is still whole
  at its point.
- **Rays are not dithered, and the two sides must not shade each other:** a stand-in leaves the shadow rays while any of its members is in the
  scene, and its own shadow rays start 4 m out (`INSTANCE_GOES`), clear of the pieces replacing it. Without that, sections of a tower went dark as they
  crossed (the owner's flight, 2026-11-08). The members cast from the moment they are in.

Measured along the walk at 2560 x 1440 under DLSS Quality, textures a level down, against the same build with hard cuts:

| | Hard cuts | Fade |
|---|---|---|
| GPU per frame | 18.88 ms | 18.99 ms |
| Frame time p50 / p99 | 18.92 / 23.79 ms | 19.00 / 23.77 ms |
| Video memory | 11,518 MiB | 11,828 MiB |
| Meshes held, most | 7,032 | 7,543 |
| Worker per edit p50 / p99 | 1.6 / 4.0 ms | 1.8 / 4.4 ms |

The pop hunt's score (#363) could not tell the two apart: its strongest pops are near objects sweeping past, the same in both runs. The frames of the two
runs differ in small patches only, 12 to 80 of 3,600 tiles by 2 levels or more, round the pieces mid-swap. Whether the swaps are now invisible is the
owner's to say: that walk is the check.

The resident district is untouched by construction (no instance is flagged, no table made). Its stills differ from those before this step by a
FLIP mean of 0.006 and 0.013 at road 1 and at the tall building, along thin edges: that comparison also spans the DLSS exposure fix pushed in
between (dec2b714), so it is not a clean 0 px.

Known limits: after a jump across the map, a stand-in dissolves before its members' cell is in, a moment of nothing until the worker's next edit. A group
spread wide now comes in earlier than its members' own ranges, at some cost in meshes.

**Costs and risks:**

- **Pops:** the swap is a hard cut until step 5, and the owner sees those at once.
- **Edits at speed:** along a straight line 1,169 pieces a second change at a walk and 3,714 at 40 m/s, 8,256 at worst, counted
  piece by piece: swapping by group will raise them somewhat. No edit load of that size has been measured; step 2 does.
- **Memory:** the distinct meshes suggest a large cut in geometry (about a fifth of them live at the busiest camera), but meshes differ
  in size, so it is a measure of step 2 and not a promise. Textures stay resident until D-105 is wired.
- **A second reference point** in the data is not understood and is not used. A piece it matters for would show as a range error.
- **Disk and time:** the conversion of about 2,000 more sectors, its meshes and textures.

**Decided** (the owner, 2026-11-08):

1. This design, starting with step 1.
2. The area first: the district's present reach, about ±1,100 m. The whole city once step 2 is measured.
3. The fade: after walking the hard swap, if it shows pops.
