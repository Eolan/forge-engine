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
| `forge-core` | built | `Seed`/`SplitMix64`, `dmath` (libm-backed), `hash` (pcg3d/pcg4d/mix64), generational `Handle`, the material record (`material`: `Material` with render, physics and tag layers, `MaterialTable`, D-007; the render layer's reflectance since #56, the ice's bubbles since #61) |
| `forge-task` | built, measured | work-stealing pool with 3 priorities, `Counter` continuations, `scope`/`join`/`par_*`, `TaskGraph`, `BlockingPool`, `Task<T>` |
| `forge-gpu` | built | `ash` Vulkan 1.3+ device (mesh shaders, ray query, min-reduction samplers, memory budget detected), acceleration structures and ray queries (`accel`: BLAS and TLAS builds, reached by device address, and a TLAS rebuilt every frame inside a graph pass, `DynamicTlas`, #79; a BLAS refitted in place every frame over positions a pass rewrites, `DynamicBlas`, #165; D-029), `gpu-allocator` with every allocation counted by category and every host write counted as upload (`memory_report`: per-heap usage and budget from `VK_EXT_memory_budget`, issue #9), RAII `Buffer`/`Image`(with mip views)/`Pipeline`/`Surface`, swapchain, Slang compiler with cache (and, since #25, the list of entries a program asks for, compiled ahead behind the loading screen after a shader change), the global bindless set (sampled/storage images, samplers), mesh, vertex and compute pipelines, indirect dispatches and indexed indirect-count draws, `DeviceOptions` (mesh shaders left off for `--force-fallback`), device selection that checks every feature it enables and the bindless limits and names what a skipped GPU lacks (#67), limits read from the device (the acceleration structures' scratch alignment, the mesh draw's workgroup total), graphics, async compute and transfer queues (`QueueKind`, #77), `Frames` (a timeline semaphore per queue, 2 in flight, GPU timestamps per submission, deferred deletion), safe `Commands`, and the **render graph** (`graph`: declared accesses → derived barriers, transient images and buffers aliased in one heap (#78, `FORGE_GRAPH_POISON` to show a stale address), per-pass profiler zones, host reads declared for readbacks, acceleration structure builds and the ray queries reading them (#79), `Custom` accesses for third-party work, a queue per pass with the batches and the waits between queues derived; D-020), and `dlss` (DLSS through NVIDIA Streamline's interposer behind the `dlss` feature: modes, render sizes, tagging graph images, evaluation inside a graph pass; D-024) |
| `forge-geom` | built | meshlet building and the cluster LOD DAG (`meshopt`; skinned meshes cooked as one level of roots with each cluster vertex's joints and weights, `skin`, #165; material sections through it, #41, D-027), 128 KiB cluster pages (`page`, D-025; a mesh with texture coordinates adds a UV stream to each cluster, 16-bit over the cluster's range, D-047), the mesh cache (`cache`), procedural meshes (asteroid, fractured chunks with their fracture faces as section 1 since #60 and #62, the city's props and terrain, a heightfield with cells drawn finer and the coarse cells around them stitched as fans, for the island's river channels, #105, and drawn in tiles that are the whole mesh to the bit, normals included, #106, the island's stones by their rock: granite's corestones, slabs and tors, limestone's bedded and jointed blocks, #130; rounded boxes, #136), `model` (glTF 2.0 models through the `gltf` crate: the binary form, its meshes in the scene's frame with their primitives as material sections, #138; `PropKind::Imported` cooks them; named empties kept as points, a joint's pivot, #143; texture coordinates, the embedded images and each material's textures with their samplers and transforms, what Forge cannot draw yet listed, D-047; `.gltf` files with the buffers and images beside them, `load_gltf`, D-048), `fracture` (a convex solid cut into the Voronoi cells of points in it, each a convex polyhedron and a mesh with its cuts as section 1, in `f64` without trigonometry so the pieces are the same bits everywhere, #142), shared GPU layouts |
| `forge-render` | built (phase 1) | `MeshletSceneBuilder`/`MeshletScene` (many meshes, instances), `MeshletRenderer` (cluster LOD DAG, instance and cluster culls in compute appending in a fixed order, drawn by mesh shaders or the indirect-count fallback (`GeometryPath`, issue #5), a software rasteriser for dense clusters (#3), two-pass HZB occlusion from the previous frame's pyramid (#33), whose pass 1 lists what it leaves to pass 2 (#92), instance occlusion and cells of 64 instances (#38), cluster pages resident or streamed through a pool (`streaming`, #36, D-025), the start view's pages loaded before the first frame (#121), movers: the instance table's last records written every frame from a ring the CPU fills, their cells and their motion vectors (#79), the visibility buffer and its resolve by material class (#20, D-026), statistics), `material` (the GPU material table, `TextureSet`, stock rock and ice rows; `ModelTextures`: a model's materials as rows, their maps read by UV with glTF's metallic-roughness model, D-047), `textures` (procedural tileable textures with mips; a model's PNG and JPEG images decoded with theirs), `mipcheck` (the resolve's texture LOD against a fragment shader's), `sky` (sky-view table, the sky's irradiance in nine SH coefficients (#47), aerial perspective, compose, the cloud layer laid over the sky (#145); D-023; the tables on the async compute queue, #77), `night` (the key light from the sun to the Moon, the reference illuminance, the Moon's disc and the stars, #164, D-046), `bloom` (the downsample/upsample chain; D-022), `gtao` (ambient occlusion from the depth, after XeGTAO; D-030), `dust` (a froxel volume of sunlit dust, shadowed by rays; D-032), `raytrace` (BLAS per mesh from its DAG, a ground in tiles cut at one error as one mesh, #106; TLAS over the instances, and the movers' own rebuilt every frame and traced after it, #79; D-029), `probes` (diffuse light from DDGI probes in cascades around the camera, updated by ray queries on the async compute queue; D-036, #77), `Taa` (jittered HDR target, motion vectors, clipped history rescaled by exposure, both compute passes so the serial frame repeats (#161), then the pass that shows the history, sharpened or not), `DlssUpscaler` (DLSS in place of the TAA resolve, the scene drawn at DLSS's input size), `Starfield` (stars, nebula, a physical sun disc, a planet under its atmosphere), `Atmosphere` (Hillaire 2020 transmittance and multiple-scattering tables as graph passes, the per-pixel march for views from space), `LuminanceMeter` + `AutoExposure` (histogram metering, EV100), `Display` + `Tonemap` (AgX, ACES fit, PBR Neutral, ACES 2.0 as run-time data), `aces2` (ACES 2.0's output transform ported from OpenColorIO and its baked table, #76), `tonecheck` (both ACES 2.0 paths on the GPU against the CPU), `water` (the sea's FFT cascades on the async compute queue, checked against `forge_procgen::Ocean`, and its surface: a clipmap drawn after the sky's compose, whose mirror and shadow rays `MeshletRenderer::trace_requested` traces, and the shore: the waves damped by the floor, trains that shoal and break, the wet sand the resolve's layered ground computes from the same swash (`shaders/shore.slang`), and the rivers: ribbons from `forge_procgen::river`, level water in the channels the island's mesh carves, flow-mapped, parted around the stones in them and around what floats in them this frame (`WaterSurface::set_floaters`, the flow relative to each, #107) and blended over the bed in the same pass, their water carried out into the sea in a plume at their mouths, and the lakes: a plane each at its level clipped to its mask, shaded as the rivers' water; a pool (#144): a simulated grid's surface, depth and flow uploaded each frame, two triangles between each four samples, faded where it thins, shaded as the rivers' water; the sea optional (`set_sea`); D-038, #105; and the wakes of what moves through the lakes and the sea, `wakes`: wave particles carrying packets of waves on the async compute queue, splatted into a field of heights around the camera whose slopes the surface adds, #107; the spray where it splashes, `splashes`: ballistic particles in a ring handed out in order, emitted and moved on the async compute queue, drawn as streaked sprites at least a pixel wide with a reactive mask for TAA, #107; and the view from under the water: the water the camera is in (the sea's, or a lake's or a river's the CPU finds), the surfaces from below with Snell's window, the water between the camera and what it meets, and the waves' caustics on the floor, which the resolve's layered ground takes from the cascades' slopes, #108); `shallow` (#162: the GPU's shallow-water layer, the column model's scheme on a finer grid, pulled towards the columns each frame and drawn as the pool, D-044); `clouds` (#145: a layer of cumulus behind `--clouds`, ray-marched at half resolution from a weather map and Perlin–Worley and Worley volumes in 2-D atlases, lit by the sun through the air and octaves of multiple scattering, blended with last frame's; their shadow map, 30 km round the camera, which the resolve multiplies into the sun's light; and the sky's light with them in it, a copy of the sky-view table with the clouds laid over it and its irradiance, for the resolve, its reflections and the probes, #163); skinned meshes (`skin`, #165): movers whose clusters' vertices a pass bends by four joints each every frame before the culls, writing the pool, the ray tracing's copy (its BLAS refitted) and the previous positions the movers' motion vectors read; every renderer declares graph passes, none writes a barrier. Next: lighting tiers (ReSTIR direct light, the probes' reflections, NRD) |
| `forge-world` | started (phase 2, 2026-09-26) | `frame` (`f64` positions in a tree of frames: sectors of an `i64` grid of 2⁴⁰ m, systems, bodies, constructs; a position restated in any frame through the lowest common ancestor, exactly across sectors; D-004), `cells` (a position as an integer cell of 1 km and an `f32` offset, the form the GPU instance table and frame block store, #93), `partition` (the flat grid and the equi-angular cube sphere cut into square cells at quadtree levels; `dmath` for the transcendentals, D-016), `cell_id` (`u64` names of those cells: kind, level, face, x, y; parents and children by arithmetic), `streaming` (the cells a viewer wants per level within a reach in cells, loads best first and unloads with hysteresis). Planned: HLOD proxies' content, cell persistence, the weather state (D-034 puts the material table in `forge-core`) |
| `forge-physics` | started (phase 3, 2026-10-02) | Jolt Physics 5.6 (D-009) vendored in `third_party/jolt` and built by `cc` with cross-platform determinism and double precision, behind a narrow, batched C layer of Forge's own (`cpp/forge_jolt.h`, after JoltC) bound by hand: shapes (box, sphere, capsule, cylinder, convex hull, static mesh, offset), bodies, the step, transforms and velocities of many bodies per call, sleep, impulses, forces, a ray cast, the whole state saved and restored, a hash of transforms to the bit (#136); `buoyancy` (#138): closed hulls cut at the water's surface, the pressure on each submerged piece, pressure and skin drag, radiation damping near the surface, after Kerner 2015, with no transcendental function; forces through points and torques for many bodies in one call; characters (#139): Jolt's `CharacterVirtual`, a capsule on its feet that steps up stairs, keeps to the ground, stops at steep slopes, rides what moves and pushes what it meets, saved and restored with the world and numbered per world; vehicles (#140): Jolt's `VehicleConstraint` with its wheeled controller (four wheels on springs found by a cylinder cast, front steering, a geared engine through a differential, brakes and a handbrake, anti-roll bars), saved with the world; a shape's centre of mass read and moved (`with_center_of_mass_at`); `aero` (#141): lift and drag on flying surfaces, each a plate in its body's frame with its incidence from the airflow, the thin-wing law to the stall then a flat plate's, induced and broadside drag, control surfaces as added incidence, no transcendental function; joints (#142): fixed and distance constraints, per joint the impulses it carried in the last step, broken and mended, their solver iterations raised for a tall stack, saved and restored with the world, whether broken or not; ragdolls (#143): Jolt's ragdoll of parts held by ball joints and hinges with their limits, a part not colliding with its parent, motors of a stiffness in N·m a radian driving it to a target pose, limp at zero torque; `shallow` (#144): D-009's authoritative column model, water depths on a grid and velocities on its faces (after Müller-Fischer 2008: semi-Lagrangian velocities, upwind fluxes scaled to what each cell holds, the surface's slope), the volume kept, saved and digested, a `Water` for the buoyancy, and what floats pushing the water aside (#151: its volume under it a thickness the slopes see). Planned: per-construct spaces, material lookup, deformation writes |
| `forge-anim` | started (phase 3, 2026-10-04) | D-012's clip runtime, in-house (#165): a skeleton (joints in the skin's order, parents, the frames of nodes between them, rest pose, inverse bind matrices) and clips (step, linear and cubic-spline keys per joint) from glTF's skins and animations, sampled into poses kept as three arrays, two poses blended by a weight, the model-space pass and the skinning matrices; no transcendental function. GPU skinning lives in `forge-render` (`skin`, below). Planned: inertialized transitions, blend spaces, IK, motion matching, contact events |
| `forge-audio` | planned | real-time thread, mixer graph, spatialiser, material-driven sounds, weather ambience |
| `forge-net` | planned | transport, replication, prediction, interest management, replay |
| `forge-sim` | started (phase 3, 2026-10-02) | the clock and the inputs (#137): a fixed tick of 1/60 s, commands stamped with their tick and applied in a fixed order, the `Simulation` trait (tick, save, restore, digest), recordings to a file and their replay against the digests, a server and clients that run ahead, predict their own commands and go back to the server's state only when a snapshot's digest is not the one predicted (D-010), an in-process link that delays, jitters and loses packets from a seed. Planned: `bevy_ecs` storage with the Forge executor, gameplay systems, simulation LOD |
| `forge-procgen` | started (phase 2, 2026-09-26) | `field` (`Field2<T>`, a square grid with a spacing; sampling, Horn's gradient, a digest), `noise` (gradient noise on an integer lattice hashed with `pcg3d`, `fbm`, ridges: no transcendental, D-016), `island` (stages 1–2 of `docs/research/terrain-genesis.md`: the mask from a distance shaping warped by noise, the uplift (lowered along trunk valleys that gather the island into a few large basins, a lake's bowl on each, #123), hardness and rain fields, an optional wind with orographic rain refreshed from the relief; the whole island as one call, cached on disk), `flow` (D8 receivers with a pinned tie-break, the depressions by the basin graph of Cordonnier–Bovy–Braun each step and by priority flood for the reference and the lakes, the downstream-first stack built by row bands in parallel, integer drainage areas, the buffers kept across steps, the basins, the alluvium's grade to the sea, #123), `erosion` (the implicit stream-power law with `n = 1` in stack order with the rain summed over the catchment, the stack's trees in parallel on `forge-task`, hillslope diffusion, lakes as depressions that fill with sediment; the same bytes with any thread count), `hydrology` (rivers as polylines with Strahler orders and widths, lakes with their level, depth and outlet), `coast` (the signed Euclidean distance to the coast, the sea floor from it, the shore smoothed within a few metres of the sea's level, #106), `river` (each river's course smoothed and settled on its valley's floor into a ribbon of points with its width, depth, speed and a level that only falls, under its banks, and the runs where a lake takes it on, #120, its delta where it runs into one, its water easing flat to the lake's level and its channel widening, #120, and the rounded corners where a tributary meets its river, #119, and on the steep reaches steps and pools, #122, the small rivers brooks of nature's size, #123, and in a large mouth at the sea bars of sand its water runs round, #127), `channel` (the channels carved under that water on cells drawn in quads of a metre, the confluences' corners rounded, the steps' lips holding their pools, the deltas' fans of sand raising the lakes' floors in front of the mouths, the outlets' flooded arms risen into sills, with the ground around them smoothed, and the stones on their beds and the steps' lips), `lake` (the lakes' water: a level plane each over its flooded depression and a sample more; their shores refined by `channel`), `ocean` (the JONSWAP/TMA directional spectrum synthesised by an inverse FFT on the CPU: heights, displacements, slopes, the Jacobian; D-038 🟡's reference; and per tick for what floats, `Ocean::displacement` and `SeaHeights`, #138: the phases only where the spectrum has energy, two fields per transform, the lines on the job system, sampled as the GPU's filter reads them), `layers` (stage 6's first rule: rock by slope and altitude; the island's geology, D-042, #129: granite in the hills, limestone on the low ground and the sea cliffs, karst on its dry slopes), `beach` (stage 6's beaches, #128: shingle on the headlands and under steep land, pale sand in the bays and by the rivers' mouths; black sand only for a volcanic island), `sites` (where the island's loose rocks lie and of which rock, #130: talus under the steep ground in the rock it fell from, tors on the granite's crests, blocks on the karst, few elsewhere; the map the GPU placement draws them from), `preview` (PNGs: height, hillshade, flow, the basins, the overview, the network by order, the coast, the sea); `tools/genesis` runs it (`docs/demos/island.md`) and `city-blocks --island` draws the field. Planned: D∞ moisture, the lake rule (#97), the ×2 amplification per tile, the material rule table and the water mask for the water pass; later ecosystems, settlements (D-039 🟡), grammars |
| `forge-app` | built | window, input, frame loop that owns each frame's `FrameGraph` (swapchain import, demo passes, overlay, capture, present), a loading screen for demos whose start-up is heavy CPU work (`run_loading`, #25: the work runs on a thread, the loading frames do not count), PNG capture, fly camera, the profiler overlay (F1: GPU and CPU zones, the memory group) and Tracy frame marks, zones and memory plots (`profiling`), the Vulkan API through Streamline when asked (`AppConfig::streamline`, feature `dlss`) and the preferred GPU is NVIDIA's (#67), else the plain loader, the HDR output (`DisplayOutput`, D-022: HDR10 or scRGB on the display, or off-screen with a preview, #94; the calibration pages and MaxCLL and MaxFALL measured from the frames shown, #125) |
| `tools/imgdiff` | built | pixel and perceptual (LDR- and HDR-ꟻLIP) comparison of captures (the golden-image check; exit code for CI) |
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
  frame's transient images (depth, HDR colour, motion vectors) and transient buffers (the
  culls' work lists, status words, visible-cluster lists and rejects, #78; their addresses
  taken in a pass body, valid from their first pass to their last) in one heap where
  lifetimes allow aliasing, records the passes in declaration order with a profiler zone
  each, and carries the final states into the next frame. A pass may ask for the async
  compute or the transfer queue (#77): the graph moves it up to just after its last conflict,
  splits the frame into batches (one submission each), and derives the timeline waits between
  queues from each resource's last writer and readers per queue, leaving out those its queue
  already made (#104). Persistent images
  (`GraphImage`: depth pyramids, TAA histories) and buffers (`GraphBuffer`: indirect
  arguments, the probes' state, readbacks) keep their state between frames; resources a frame
  in flight may still use go through `Frames::destroy_later`. Nobody outside `forge-gpu`
  records a barrier.
- **Audio thread**: real-time priority, never touches the pool, communicates by lock-free
  queues.
- **Network thread**: `tokio` runtime for I/O only; packets are handed to the simulation.
- **GPU**: a graphics queue, and an async compute and a transfer queue when the device has
  them (#77, D-020; `FORGE_ASYNC=0` keeps everything on graphics). The city's sky tables and
  probe update run on the compute queue beside the geometry, its streamed page copies on the
  transfer queue. Resources shared by queues are `CONCURRENT` (render targets stay on
  graphics), so no ownership transfers. A frame ends on the graphics queue after every other
  batch; two frames in flight on timeline semaphores; the CPU never waits on a whole queue.
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
(`forge_core::material`) has a render layer. Its GPU form is a 96-byte row of the material
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
  In the city the rays aim within the sun's disc, and TAA averages them into soft shadows
  (#54).
- **Sky light and its occlusion** (issues #47, #48; D-023's note, D-030): under a sky, a
  surface also takes the sky's irradiance for its normal (nine SH coefficients a frame,
  from the sky-view table), scaled by GTAO computed from the depth before the resolve.
  It also reflects the sky: Schlick's Fresnel over the sky-view table in the mirror
  direction (issue #49, D-031), and on the smooth rows the city itself: a mirror ray against
  the TLAS, the hit shaded from the BLAS cut kept on the GPU (issue #50).
  A row's reflectance (F0, issue #56) sets how much it mirrors: coated glass 0.3.
- **Diffuse light from probes** (issue #53, D-036): in the city, cascades of DDGI probes
  around the camera, traced by ray queries every frame (the sun through a shadow ray, the
  probes' own light for one more bounce, the sky-view table for misses), replace the sky's
  irradiance on the diffuse side. They bring the street's occlusion and the city's bounce
  light; beyond them the sky's irradiance takes over, and GTAO still marks the contacts.
- **Ground in layers** (issue #42, D-028): a `layered` row names a layer map, a byte a texel,
  and each layer is the standard row after it; the layered pass blends the two heaviest
  layers around each pixel. A row's contour (`RenderLayer::contour`, #106) draws one layer by
  the ground's height under the pixel instead of the map's texels: the island's sand under
  2.5 m.

**Dense clusters go to a software rasteriser** (issue #3). The cluster cull marks a cluster
dense when it is in front of the near plane, under 64 pixels across and has fewer than two
pixels of its bounding sphere's screen rectangle per triangle: there the hardware's fixed
cost per primitive dominates. In both passes (since #30, so that occlusion changes no pixel),
when the renderer runs it, those clusters go to a second raster list instead of the hardware's: a compute workgroup per cluster
transforms and snaps its vertices exactly as the fixed-function stages do (perspective
division, the viewport, round-to-nearest-even to 1/256 pixel), then a thread per triangle
culls back faces, walks the pixel centres of its bounding box with 32-bit edge functions and
the top-left rule, interpolates the depth linearly in screen space and keeps
`depth << 32 | id` with a 64-bit atomic maximum, only where it beats the hardware's pixel of
that pass (nearer, or at equal depth the larger id: the hardware's depth test keeps the last
drawn and it draws in id order, so every path resolves ties alike). A merge (an indirect
draw: a rectangle per software cluster, or one full-screen triangle when they are many, #32;
empty when no cluster went to software) writes those samples into the
visibility buffer and the depth and clears them; everything after the draw, from the depth
pyramid to TAA, sees one image. The renderer runs it (`SwRaster::Auto`) when the recent
frames held 1.5 M dense triangles or more, until they fall below 0.75 M: the raster pass and
the merge cost about 0.02 ms, which a million dense triangles repay. It needs 64-bit buffer
atomics; without them every cluster is drawn in hardware.

**Colour is physical and pre-exposed** (issue #7, D-022). Lights carry photometric units
(the sun in lux, its disc in cd/m² from its solid angle) and every pass writes luminance
multiplied by the frame's exposure, so the HDR targets stay near 1 in fp16 whatever the
scene. The exposure is an EV100: fixed, or automatic from a 256-bin log-luminance histogram
of the finished HDR image (`exposure/luminance histogram`: clear, count in shared then
device-local memory, copy 1 KB to a cached per-slot readback, `HostRead`), read two frames
later and followed on the CPU with separate speeds up and down. Temporal passes rescale
their history by the exposure ratio. The display transform is chosen at run time from
`tonemap.slang` (AgX, ACES fit, Khronos PBR Neutral, and ACES 2.0's output transform through
a baked 65³ table, #76, with the per-pixel transform as its reference) and applied where the last HDR pass
writes the display image (the pass that shows TAA's history in the ballad, the stand-alone display pass
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

**The night is the same machinery with the Moon as the key** (D-046, #164, `forge_render::night`).
With a time of day (`--day`, `--time-of-day`), the renderer's sun fields carry the key light:
the sun, then once it is 3° under the horizon the Moon, at its measured illuminance, so its
shadows are traced and the probes see it as they saw the sun. The sky's tables are stored per
unit of a reference illuminance that follows the scene's light (128 klux at noon, a few tenths
of a lux under the Moon), so they keep fp16's precision at night; each light is weighed against
it, the sun's twilight being the second light under the Moon. The sky-view table runs round
the whole circle of azimuth for the two lights. The compose adds the Moon's disc (lit by phase),
stars binned on a cube of cells round the sky (procedural, or a catalogue's), the Milky Way and
airglow. The exposure is held a few stops under the eye's adaptation at night, and the display
pass fades colours and tints them blue as the rods take over (the Purkinje shift).

**The resolve is TAA or DLSS** (issue #8, D-024). The scene is drawn jittered into a
pre-exposed HDR target either way, and the motion vectors (UV offsets from depth and the two
cameras) are their own pass. TAA resolves at the output size and writes the display image
through the tone curve in the same pass, or a sharpening pass does (RCAS, D-045). DLSS (the
`dlss` feature, on by default, on an RTX GPU, the Vulkan API through Streamline's interposer)
draws the scene at its input size, upscales into an HDR image at the output size, and the
display pass applies the curve and bloom. DLAA is the default of interactive runs where it runs
(D-045); scripted runs keep to TAA, whose images repeat to the bit. The DLSS pass
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
