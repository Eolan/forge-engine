# Forge — Roadmap

Systems are built in the order below, each closed by a demo with numbers (`docs/demos/`),
tests, and decisions (`DECISIONS.md`). Phases overlap where crates are independent; nothing
in a later phase may be started before the decision it depends on is accepted.

## Checkpoint 1 — 2026-09-24 (this one)

Delivered: research files for task systems, GPU geometry, large worlds, lighting (+ the
files still being written: physics, netcode, audio, vegetation/materials, animation, memory);
`forge-core`, `forge-task` (measured), `forge-gpu`, `forge-geom`; demos `task-bench` and
`meshlets` (127 M-triangle scene, two-pass occlusion, 1.1 ms GPU, pixel-identical to brute
force); `tools/imgdiff`; ARCHITECTURE, DECISIONS, this roadmap.

**Decisions taken 2026-09-24:** the owner accepted D-006 to D-014, D-018 and D-019 as
recommended (details in `DECISIONS.md`); D-008 ships with hardware ray tracing required for
players. D-019 came with a wider brief — Earth-like planets with biomes, biome and weather
transitions and ecosystems — which queues a planetary-environment research file. The table
below is kept as the record of what was asked:

| Decision | Recommendation | Blocks |
|---|---|---|
| D-006 ECS | `bevy_ecs` storage + Forge executor | simulation, netcode |
| D-007 material record | the unified row as specified | physics, audio, rendering, weather |
| D-008 RT policy | hardware RT required for players, raster tier for tools only | lighting phase scope |
| D-009 physics engine | see physics-fluids.md (Jolt expected; box3d tracked) | physics, animation |
| D-010 transport | see netcode.md (QUIC datagrams expected) | netcode |
| D-011 audio stack | see audio.md | audio |
| D-013 vegetation ladder | mesh → cards → octahedral → voxel aggregate → canopy | forest demo |

## The living showcase: `asteroids` (the ballad)

A scripted flight through a dense asteroid field in space, kept alive across every phase.
It always shows the best the engine can do at that moment and carries its own profiling
(Tracy zones, GPU timestamps, on-screen statistics, `--capture` for golden images):

- Phase 0 ✅: meshlets, task/mesh shaders, two-pass occlusion, temporal anti-aliasing, a
  procedural starfield with a planet and the sun, rock and ice asteroids, thousands of
  instances of seven procedural meshes, a spline camera path, Tracy hooks. Culling is
  verified pixel-exact against brute force by the A/B harness (`--no-occlusion`,
  `--no-cone`, `--show-culled`, `imgdiff`), which found and closed two silent culling bugs.
- Phase 1: cluster LOD DAG ✅ and instance cull pass ✅ (2026-09-24: 78 M → 0.6 M
  triangles, GPU 5.5 → 0.34 ms, pixel-exact A/B), profiler overlay ✅, render graph ✅
  (2026-09-24: every pass declared, every barrier derived, transients in one heap,
  pixel-identical), the sky drawn last behind the rocks ✅ (0.33 → 0.28 ms), the
  visibility buffer ✅ (shading once per pixel in compute, 0.30 ms), physical light units
  with automatic exposure and run-time tone curves ✅ (0.31 ms), the planet under a Hillaire
  atmosphere ✅ (the limb, the terminator and the sun through the air from one integral;
  0.325 ms, 0.34 with the planet in view), DLSS through Streamline as an option next to
  TAA ✅ (U switches mode at run time; 0.60–0.77 ms, the DLSS pass itself 0.45 ms at
  1600×900), memory counters in the overlay ✅ (VRAM against the OS budget, the engine's
  allocations by category, uploads per frame; 359 MiB of a 14.9 GiB budget), the software
  rasteriser for dense clusters ✅ (issue #3: off at the ballad's 1 px LOD, where it would
  not pay; full detail 4.37 → 2.02 ms), cluster pages ✅ (issue #36: 16-byte vertices in
  128 KiB pages, 0.316 ms; streamed in city-blocks), rock and ice as rows of the material
  table, shaded by class ✅ (issue #20, D-026: 0.34 ms), bloom ✅ (issue #44: 0.387 ms), the
  sun's ray-traced shadows between the rocks and the textured rock ✅ (issue #46, D-029:
  0.444 ms), GTAO on the fill ✅ (issue #55, D-030: 0.554 ms; soft shadows opt-in, since
  TAA smears their wide penumbrae in motion).
- Phase 3: physics — asteroids tumble and collide; **collisions and laser or missile damage
  break them according to their mass** (Voronoi fracture into debris, support graphs for the
  big ones), with proper impulses on every piece.
- Phase 4: the lighting tiers — sun with ray-traced shadows, reflections on ice and metal,
  volumetric dust and the nebula lit by the sun, path-traced reference frames.
- Phase 5–7: **space ships in pursuit of other ships**, firing lasers and missiles,
  destroying each other and sometimes crashing into asteroids; a second player flying
  alongside over the network; spatial audio for thrusters, impacts and explosions; ship
  animation (thruster gimbals, damage states).
- Reference look (owner's references): dense belts against a planet or a sun with a bright
  halo, volumetric light between the rocks, ice asteroids in blue, dark rock in the
  foreground with lit rims, asteroid bases carved into the big ones later on.

## Phase 0 — Foundations ✅

- `forge-core`: seeds, deterministic math, hashes, handles.
- `forge-task`: job system. Demo `task-bench`.
- `forge-gpu`: device, memory, swapchain, Slang, bindless set, pipelines, frames, commands.
- `forge-geom`: meshlets, procedural meshes. Demo `meshlets` (task/mesh shaders, HZB).
- `tools/imgdiff`, docs skeleton.

## Phase 1 — Render core (next)

**First, a profiling overlay on screen (owner's request, 2026-09-24) ✅:** F1 in every demo
cycles a compact and a full view of GPU time per pass from named timestamp zones
(`Commands::mark("group/name")`) and CPU time per zone of the frame loop, each with
milliseconds and a bar as share of the frame, grouped by subject and foldable with the digit
keys, plus totals, p50/p99 and the demo's counters (`forge-app` `overlay`/`profile`; text
from a TTF atlas — JetBrains Mono by default, any font via `FORGE_OVERLAY_FONT` — with a
built-in pixel font as fallback; no UI dependency). The same zones feed Tracy's GPU timeline
under `--features profiling`. `docs/PROFILE.md` mirrors the overlay at each checkpoint with
a verdict per item on what is expensive and how to attack it.
Its memory group (issue #9) ✅ shows each heap's usage against the budget
`VK_EXT_memory_budget` reports (red past 90 %, the reserve D-018 keeps), the engine's
allocations by category, the memory outside the allocator and the upload and readback
traffic per frame.

Goal: the renderer skeleton every later system draws through.

1. `forge-app`: window, input, frame loop, capture, debug overlay (egui), shared by demos.
2. Render graph ✅ (2026-09-24, `forge-gpu::graph`, D-020): passes declare reads/writes,
   barriers derived, transient images aliased in one heap, deferred deletion by frame slot,
   a profiler zone per pass. Still to come on it: async compute and transfer queues,
   transient buffers, parallel recording of pass bodies. (Lesson from the previous project:
   undeclared buffer uses made NVIDIA replay stale indirect arguments.)
3. Compute culling shared by both paths + `vkCmdDrawIndexedIndirectCount` fallback ✅
   (2026-09-24, issue #5): an instance cull and a cluster cull per mesh pass append the
   visible clusters in a fixed order; mesh shaders or one indexed draw per cluster draw
   them, 0 pixels apart (`--force-fallback`). Mesh path at the task path's cost, fallback
   draw about 2× (numbers in `docs/demos/meshlets.md`).
4. Cluster LOD DAG (meshoptimizer `clusterlod` through FFI), GPU LOD selection by
   screen-space error, streaming of cluster pages from disk through a GPU request buffer
   ✅ (issue #36, 2026-09-25, D-025: the culls write each page's need, the pages stream
   through a pool with residency closed upwards; `docs/demos/city-blocks.md`),
   software rasteriser for sub-pixel clusters (64-bit atomics) ✅ (issue #3, 2026-09-24:
   dense clusters of the first pass in compute, merged into the hardware's targets; auto
   when a frame holds enough of them; full detail 2× faster, `docs/demos/meshlets.md`).
   Metrics: DAG roots reached, cluster fill.
5. Visibility buffer ✅ (2026-09-24: the mesh passes write `visible cluster << 7 | triangle`
   next to the hardware depth, a compute resolve reconstructs the attributes with analytic
   barycentrics and shades once per pixel; `docs/ARCHITECTURE.md` §4). Material
   classification ✅ (issue #20, 2026-09-25, D-026: the D-007 table on the GPU, the standard
   pass listing 8×8 tiles for one dispatch per other shading class, procedural textures projected
   triplanar and sampled with the reconstructed derivatives, within 0.06 of a level of a
   fragment shader's). Material sections within a mesh ✅ (#41, D-027: a building's
   windows are glass). Terrain layers ✅ (#42, D-028: streets, sidewalks,
   plazas and rocky slopes from a layer map). The software rasteriser's
   64-bit depth|id samples are merged into it (#3).
6. HDR pipeline ✅ (2026-09-24, D-022): physical light units (the sun in lux, its disc from
   its solid angle), pre-exposed fp16 targets, histogram exposure with EV100 adaptation,
   AgX / ACES fit / Khronos PBR Neutral switchable at run time, golden captures per curve.
   Bloom ✅ (issue #44: the downsample/upsample chain of Jimenez 2014 before the tone
   curve, 0.04 ms at 900p). Still to come: ACES 2.0's output transform, a perceptual golden-image
   metric.
   Hillaire atmosphere ✅ (2026-09-24, D-023: transmittance and multiple-scattering tables
   as graph passes, the per-pixel march for planets seen from space; the planet-view table
   #26 for big planets; the sky-view table, the aerial perspective and the sun seen from the
   ground ✅ with city-blocks, issue #43; the sky's irradiance on the shaded sides ✅, nine SH
   coefficients a frame, issue #47; its ambient occlusion by GTAO ✅, issue #48,
   D-030). TAA ✅. DLSS ✅ (2026-09-24, D-024: Streamline's interposer behind the `dlss`
   feature, TAA the default, every mode switchable at run time, the LOD error in output
   pixels).
   **Demo:** `city-blocks` — a million GPU-placed instances of twenty props with the DAG,
   120 fps at 1440p on the 5070 Ti, 60 fps on the 3080, flying at 300 m/s with streaming on.
   In steps (`docs/demos/city-blocks.md`): the renderer at a million instances ✅ (#33), the
   twenty props cooked and cached ✅ (#34), terrain and GPU placement ✅ (#35: a million instances
   in 4.96 ms), culling at scale ✅ (#37: 1.06 ms, instance occlusion next in #38),
   streaming ✅ (#36, D-025: 128 KiB cluster pages, the flight at 300 m/s through a 48 MiB
   pool with no holes), the flight at 1440p ✅ (#13, 2026-09-25: 1.64 ms GPU with TAA, worst
   frame 2.58 ms on the 5070 Ti; the 3080 run is #39), materials ✅ (#20: brick, plaster,
   concrete, glass, grass and rock, textured; the flight 1.79 ms).

## Phase 2 — World

1. `forge-world`: reference frames (`f64`), integer sector grid, cube-sphere and flat-grid
   partitions, cell streaming with HLOD proxies, `u64` cell ids.
2. Terrain: lift genesis (uplift, stream-power erosion, hydrology, priority flood) and the
   cube-sphere CDLOD from the previous projects; volumetric near field (dual contouring /
   Transvoxel) for overhangs and caves; SDF bricks for edits.
3. Water surface: FFT ocean far, flow-mapped rivers, shore handling.
   **Demo:** `island` — a 16 km island from seed, orbit-to-ground on the planet variant,
   golden shots at four times of day.

## Phase 3 — Simulation, physics, materials, weather

1. `forge-sim`: `bevy_ecs` + Forge executor, fixed tick, frame packet to the renderer,
   simulation LOD, determinism digests.
2. `forge-physics`: bind the chosen engine (D-009), one space per construct, terrain and
   cluster collision from the same data, character controller, vehicles, buoyancy.
3. Materials in practice: friction/restitution/sound/tags from the material row; deformable
   layer for snow, sand, mud (footprints, wheel tracks) written by contacts and rendered by
   displacement; weather state mutating wetness/frost/snow.
4. Fluids: shallow-water heightfield near the player coupled to rigid bodies; particles
   for splashes; the mid/far tiers from Phase 2.
   **Demo:** `materials-yard` — walk from brick to wood to sand to snow to an ice lake;
   footprints, slipping, sound and wetness change from one table; push crates into water.

## Phase 4 — Lighting tiers

1. T1 hybrid: ray-query DDGI probes, ReSTIR direct lighting, hybrid reflections (the sky's
   term ✅, issue #49, D-031: Fresnel-weighted from the sky-view table; mirror rays in the
   glass ✅, issue #50), NRD. Ahead of the probes: the sky's irradiance ✅ (#47) and GTAO ✅ (#48, D-030).
2. Shadows: the sun's by ray query ✅ (issue #45, 2026-09-25, D-029: a BLAS per mesh from a cut
   of its DAG, a TLAS over the city's million instances, 0.14 ms of rays at 1440p; the
   ballad's rocks in #46, 0.03 ms); soft shadows ✅ (issue #54: the sun's disc over TAA's
   jitter, 0.02 ms at 1440p); structures that follow streaming and motion next.
3. Clouds (Nubis-style), froxel fog, night sky; weather rendering (rain, snow, lightning,
   wet surfaces) driven by the shared weather state.
4. T2/T3: ReSTIR GI, radiance cache, Ray Reconstruction, path-traced reference with cluster
   acceleration structures.
   **Demo:** `dusk-town` — seeded coastal town, dusk to night, 20 k emitters, a storm front,
   rendered on every tier with the path-traced tier as the reference image.

## Phase 5 — Netcode

1. `forge-net`: transport (D-010), versioned protocol, connection tokens, link conditioner.
2. Replication: snapshots, delta compression, quantisation, priority accumulator,
   cell-based interest management, authority handoff between workers.
3. Prediction and reconciliation (input queues that wait rather than repeat), lag
   compensation, replays.
   **Demo:** `hundred-bots` — 100 headless bots over a 100 ms / 2 % loss link on the server
   PC, two workers with a boundary across the island, bandwidth per client measured.

## Phase 6 — Audio

1. `forge-audio`: real-time thread, mixer graph, streaming decode, voice management.
2. Spatialisation and occlusion (D-011), material-driven footsteps and impacts, weather and
   medium ambience, underwater.
   **Demo:** `forest-to-cave` — walk from a forest into a cave in the rain; occlusion,
   reverb and material sounds change without any scripting.

## Phase 7 — Animation

1. `forge-anim`: clips, blend graph, compression, GPU skinning, IK (feet, hands, look-at).
2. Motion matching for the player; powered ragdolls tracking poses; hit reactions;
   contact events to the material layer.
3. Generated creatures: gait synthesis for procedurally generated bodies.
   **Demo:** `rough-ground` — a biped and a generated hexapod cross rough terrain, get
   shoved by a physics object and recover; footprints in snow.

## Phase 8 — Vegetation and authored materials

1. Tree growth (space colonisation, self-organising), bark and leaf materials, wind.
2. Rendering ladder (D-013) with measured distance bands; GPU placement rules.
3. Trim sheets and regional material sets for buildings; shape grammars picking trim
   regions; decals and layering.
   **Demo:** `four-km-forest` — a 4 km forest at 120 fps with the full ladder and a village
   built from one regional trim set.

## Phase 9 — Memory and streaming

1. Allocators (frame arenas, pools, GPU sub-allocation policy), budgets and telemetry.
2. SSD streaming: async I/O, GPU decompression, page-granular residency for clusters and
   virtual textures; Resizable BAR upload paths.
   **Demo:** fly-through at 300 m/s with residency and bandwidth graphs.

## Phase 10 — The games again

Rebuild `tropical-island`, `world` and `shooter` on Forge, in that order, reusing their
generation rules and lessons.
