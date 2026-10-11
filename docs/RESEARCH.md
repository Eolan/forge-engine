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
| [research/lighting-gi.md](research/lighting-gi.md) | GI tiers, path tracing, shadows, sky, upscaling | 45 | done (weather rendering: planet-environment.md §4) |
| [research/physics-fluids.md](research/physics-fluids.md) | rigid bodies, engines compared, characters, destruction, water | 45 | done |
| [research/particle-fluids.md](research/particle-fluids.md) | real-time particle liquids: SPH, PBF, FLIP/APIC, MLS-MPM and PB-MPM compared; GPU sort, scan and fixed-point atomics on Vulkan; drawing the liquid (screen-space, grid ray-march, underwater, the medium transition, glass, caustics); engines' fluids; Jolt coupling; determinism | 67 | done (#155, 2026-10-03; the decision taken in D-044 on the same day) |
| [research/night-sky.md](research/night-sky.md) | a night sky over the ground: the Moon and its light, stars (catalogue or procedural), airglow, measured light levels, low-light tone mapping (Purkinje) | 8 | short, D-046 ✅ (2026-10-04) |
| [research/shadow-denoising.md](research/shadow-denoising.md) | denoising the sun's ray-traced soft shadows: why one ray and TAA fail 8 stops under the sun, NVIDIA's SIGMA, AMD's FidelityFX denoiser, penumbra filters, licences and a patent | 15 | short, D-049 ✅ (2026-10-04), built in #172 and #173 |
| [research/netcode.md](research/netcode.md) | transport, replication, prediction, server topology | 48 | done |
| [research/audio.md](research/audio.md) | mixer, spatialisation, propagation, synthesis, middleware | 46 | done |
| [research/vegetation-materials.md](research/vegetation-materials.md) | trees, impostors, grass, trim sheets, unified materials, deformation | 50 | done |
| [research/animation.md](research/animation.md) | clips, motion matching, IK, physical characters, generated creatures | 42 | done |
| [research/memory-streaming.md](research/memory-streaming.md) | allocators, Resizable BAR, SSD streaming, residency | 40 | done |
| [research/procedural.md](research/procedural.md) | terrain, grammars, noise, ecosystems, settlements, DOD (from the previous projects) | ~90 | done, carried over |
| [research/planet-environment.md](research/planet-environment.md) | climate bake, biomes and ecotones, ecosystems, weather rendering, the environment state model | 51 | done |
| [research/data-driven.md](research/data-driven.md) | data model, reflection, asset ids, packages and load order, scripting, Wasm, hot reload, modding | 49 | done |
| [research/water.md](research/water.md) | ocean spectra and FFT cascades, shores and shallow water, rivers and lakes, water shading, the genesis hand-off, engines' water systems; §7 splashes and spray (2026-10-02, #107: emission physics, particles under TAA, coupling back) | 34 + 30 | done (Phase 2 item 3, `island`) |
| [research/rivers.md](research/rivers.md) | rivers integrated into terrain: engines' and games' river tools, procedural river research 2013–2026, the geomorphologists' channel and floodplain numbers, shallow-water simulation and its cost, a recommendation for the island's rivers | 52 | done (#112, 2026-10-01; D-040 pending) |
| [research/city-generation.md](research/city-generation.md) | road networks and hierarchy, blocks and lots, districts and landmarks, buildings from grammars and kits, interiors, a city through the cluster DAG, engines' city pipelines; extended 2026-10-10 (§7–§19): the real coast and river from elevation and hydrography data, waterfronts and rivers, parks and green infrastructure, squares and play streets, transit-first streets (Nantes, Strasbourg, Bordeaux, Pontevedra, Ghent), the vehicles a car-free city still needs, block types and mixed use, the style bible as data, the near-future city (integrated technology, well-being, no fossil fuels), interiors by building use at game rates, the city in numbers, reference cities and games, patents | 44 + 69 | done (issues #85, #86; D-039 ✅; the extension for the owner's city question of 2026-10-10) |
| [research/hdr-output.md](research/hdr-output.md) | HDR display output: Vulkan's swapchain colour spaces and metadata, Windows' composition and reference white, PQ and 10-bit dithering, ACES 2.0's HDR presets and OCIO's builtins, the engines' display mappers, UI at paper white, calibration, verification without an HDR monitor | 27 | done (issue #94; sources on GitHub read, the rest confirmed by search only, #99) |
| [research/render-graph-next.md](research/render-graph-next.md) | render graph, next: transient buffers (lifetimes, aliasing rules, heap packing bounds, device addresses and what validation cannot see), parallel recording of pass bodies (pools per worker, chunks, barriers per command buffer, one submit per queue), split barriers and events against the queues, when it pays and how to measure it | 22 | done (issue #78; Granite, the spec sources, the samples and the layers read on GitHub, the vendor and engine pages confirmed by search only, #99) |
| [research/planet-terrain.md](research/planet-terrain.md) | planet terrain from orbit to the ground: the cube sphere and its alternatives (S2, Cesium's quadtree, HEALPix), LOD on the sphere (spherical clipmaps, CDLOD, concurrent binary trees, Nanite's landscape, Forge's DAG per tile), precision, horizon culling, skirts and pops, tiles generated on demand and streamed through the page pool, the planet games and the open source, the atmosphere from orbit, what to measure | 32 | done (Phase 2's planet variant of `island`, D-037 ✅; the Cesium formats, the CBT demos and the Godot planets read on GitHub, the rest confirmed by search only, #99) |
| [research/worlds-at-every-scale.md](research/worlds-at-every-scale.md) | worlds at every scale: No Man's Sky (Worlds Part I/II), Elite's Stellar Forge and Odyssey, Star Citizen's Planet Tech and Genesis, Space Engineers, KSP2, SpaceEngine, Starfield, Enshrouded, Valheim, Minecraft, MSFS 2024; large maps (the Witcher 4 demo's Nanite Foliage and PCG, Battlefield 6, Horizon, Tsushima, Far Cry 5, Zelda); tectonic planets, diffusion on a quad sphere, erosion-consistent amplification, ecosystems; voxel meshing, sparse storage and edit logs; GPU generation under D-016; a recommendation per scale (a planet atlas, biomes as rules, places, edits as a log) | 34 | done (#220, 2026-10-10: the owner's question of 2026-10-09; sources read or search-recorded, graded per entry) |
| [research/civilisations-styles.md](research/civilisations-styles.md) | civilisations and styles: vernacular form as parameters (climate, materials, culture), style grammars per culture and era (Palladio to CGA's Pompeii and Puuc Maya, Islamic patterns and muqarnas, Gothic tracery, skyscrapers, brutalism, greebles, space habitats), decor and props (Infinigen Indoors, ShapeAssembly, layout solvers, set dressing), materials by mathematics and by example, weathering and ageing, the games' culture sets, the `Civilisation` record over D-039's style sets | 46 | done (the owner's brief behind #81–#91; five READMEs read on GitHub, the rest confirmed by search only, #99) |
| [research/game-ai-ecosystems.md](research/game-ai-ecosystems.md) | game AI and ecosystems: decision architectures (behaviour trees, utility over needs, GOAP and HTN planners, schedules and packages, smart objects, the director and barks, where learned policies stand), navigation (Recast's tiles rebuilt where the terrain changes, Polyanya, flow fields, ORCA, Reynolds' steering, sparse voxel octrees for flight, lanes for traffic), crowds and the simulation LOD (AC Unity's 40/120/10 000, A-Life and Stalker 2's lesson, Census), animals and their populations (needs, senses, herds, territories, Wolf Sheep Predation and ODD), ship AI in 6 DoF (Elite, FreeSpace, Isaacs, formations, convex landing), the planet-scale layers with their hand-offs and digests, the Rust crates | 41 (+ a table of 16 projects) | done (the owner's brief behind #84, #87, #90 and #80; Phase 3's `forge-sim`; thirteen repositories read on GitHub, the rest confirmed by search only, #99) |
| [research/smart-systems.md](research/smart-systems.md) | smart systems: predictive streaming and prefetching (Unreal's streaming sources and cinematic prestreaming, DirectStorage and the PS5's priorities, learned viewport prediction against extrapolation), importance for what is drawn and what is updated (screen-space error budgets, dynamic resolution, VRS, the Significance Manager, the Animation Budget Allocator, Mass, population budgets), scheduling under a frame budget, what is public about GTA VI and Rockstar, learned models on the client (DLSS 5, NTC, neural caches, ML deformers, learned motion) and their hardware path (cooperative matrices and vectors), learned prediction and control (directors, test bots), the server (interest management, server meshing, autoscaling, VACnet), determinism and trust under D-016 | 41 sources (+ earlier files') | done (#222, 2026-10-10: the owner's question of that day; sources read or search-recorded, graded per entry) |
| [research/galaxy-sky.md](research/galaxy-sky.md) | a galaxy's sky: how many stars an eye sees (the counts per limiting magnitude, Crumey's threshold model, about 8 in space against 6.5 on Earth, where the naked-eye stars lie), how Elite Dangerous, SpaceEngine, Celestia, Gaia Sky, OpenSpace, No Man's Sky, Star Citizen, KSP and Starfield build their skies, the magnitude-limited query over a brightness-sorted tree, points near and a volume far, one sky per system, integer ids and positions under D-016, and the eye, a bridge telescope and the galaxy map as one query | 38 sources, a toy galaxy | done (#223, 2026-10-10: the owner's question of that day; sources read or search-recorded, graded per entry) |
| [research/assets-and-licences.md](research/assets-and-licences.md) | assets and their licences: four licence classes (redistributable, shippable only, engine-locked, non-commercial) and the clauses that decide them; humans (MetaHuman outside Unreal and its MIT face-rig runtime, MPFB, NAVER's Anny, Meta's MHR, NVIDIA's SOMA, the SMPL family, Character Creator, Daz, Rocketbox, clothing); what a Forge human generator would need; building kits (Quaternius's 2026 licence, Kenney, KayKit, Poly Haven, ambientCG, Sketchfab, BlendKit, Fab and Megascans, Synty, KitBash3D, TurboSquid, CGTrader); interiors, vegetation (SpeedTree's rights ending with the subscription, The Grove), vehicles, PBR textures; a paid shortlist with prices | 42 | done (2026-10-10: the owner's showcase brief, answers part of #231; sources read, read through a reader proxy or search-recorded, graded per entry) |
| [research/population-simulation.md](research/population-simulation.md) | a persistent population of millions: tiers by what a person is doing (records advanced by events for everyone, coarse agents near players, full agents close, the drawn crowd) and hand-offs without inconsistency; events against ticks, time warp; published throughput (MATSim's Hermes, SUMO, FLAME GPU 2, MANTA, LPSim, Mass, DOTS); bytes per person and totals at 1 M and 4 M; day plans, appointments, needs and personalities; Vuntra City, Cities: Skylines II, SimCity 2013, Census, X4, EVE, Nivalis Nights and others; the server and clients (interest, bandwidth, persistence, a second server); what an 8-thread server with an RTX 3080 holds; patents (Improbable, Take-Two, Activision) | 31 | done (2026-10-10: the owner's showcase brief; sources read or search-recorded, graded per entry) |
| [research/procedural-content.md](research/procedural-content.md) | procedural content: what Forge generates today and why it looks primitive; materials (Substance Designer, Material Maker, ArmorPaint, wear from geometry, example-based and neural synthesis, trims and decals, cooked against run time); models (Blender Geometry Nodes headless, Houdini Indie and SideFX Labs, Infinigen and the papers it lists, part programs for furniture and vehicles, SDFs, trees and rocks, cleanup and UVs into the cluster DAG); structures (buildings with interiors, streets, bridges, quays, green roofs and planted facades); generative 3D AI and its licences (TRELLIS.2, SPAR3D, Hunyuan3D's EU exclusion, the services); a quality ladder; patents (Adobe's Substance and PatchMatch, Imagination's patch synthesis; image quilting and marching cubes expired) | 38 | done (2026-10-10: the owner's question of that day; sources read or search-recorded, graded per entry) |
| [research/trim-sheets.md](research/trim-sheets.md) | trim sheets, thin-detail aliasing and street-aligned paving: why louvres, railings and mullions shimmer past 30–40 m (sub-pixel geometry under TAA; a cluster DAG does not prefilter), trim sheets and kit-baked far forms, wire AA, specular AA (Toksvig, LEAN/LEADR, Kaplanyan), TAA and sharpening; paving UVs per region, a pattern library, border strips at every junction; patents (NVIDIA's hashed alpha, ARM's virtual texturing; parallax expired) | 38, 247 | done (2026-10-11: the owner's walk; sources search-checked, one forum-only claim marked) |
| [research/building-structure.md](research/building-structure.md) | building structure for consistent, enterable and later destructible buildings: real wall thicknesses by era (old rubble stone 45–80 cm, Haussmann, interwar brick, post-war and new concrete, CLT, party walls, partitions), slabs and clear heights, French accessibility (lifts from R+3 and in ERP, EN 81-70 cars, stairs, corridors, doors), dwelling sizes and gross to net (≈ 0.82; 40 m² gross a person = INSEE's 32.4 m² net), a low-floor tram's body; enterable buildings in games (Bethesda's kits, procedural floor plans, interior mapping); destruction (Battlefield 6, The Finals, Siege, Red Faction, Teardown, Chaos, Blast, what Jolt offers); patents (Sony's tile stress to 2030, Celsys's room swapping, Bosch, EA, Take-Two) | 248 | done (2026-10-11: the owner's notes on wall and slab thickness and destruction; sources read or search-recorded, estimates marked) |
| [research/trip-distances.md](research/trip-distances.md) | where people work and go out, real distances for the population model: French home-work distances (INSEE: median 7.8 km, 5.4 km in dense areas, Paris residents 4.8 km; Munich MiD), distances and shares by purpose (MiD medians, Lyon, Marseille), walking shares, trip chaining (15–24 %), time of day and weekends, destination choice by distance and appeal (mobiTopp's logit, blue space); what the model got wrong (commutes too short, four nearest too local, no chaining) and starting parameters | 250 | done (2026-10-11: the owner's notes on outings; sources read or search-recorded, estimates marked) |
| [research/cosy-builders.md](research/cosy-builders.md) | consistent, organic buildings in cosy builders: Tiny Glade (no grid, a generator per element chained, kinds that change with room and context, glue between elements, windows as chameleons, local recompute, organic rules), Townscaper (irregular grid, corner modules, WFC with priorities), Summerhouse, Outside the Blocks, Dystopika, Belle Citique; split grammars, straight-skeleton roofs, constraint-based facades; what a realistic city can take (1D bay layouts with fallback chains, corners first, glue at junctions, roofs from weighted skeletons, openings as shared data, local seeded regeneration); patents (CMU's shape-grammar ones expired) | 247, 248 | done (2026-10-11: the owner's request; developers' diaries and posts, inferences marked) |
| [research/people.md](research/people.md) | people: the human runtime (MetaHuman DNA and RigLogic, now MIT; skinning at scale for #168; skin, eyes, teeth, hair cards and strands), LODs from hero to crowd (skeletal near, bone-animation textures and pre-posed shadows for thousands, impostors to 1.5 km), what AC Unity, Hitman, Watch Dogs: Legion and the Matrix Awakens did; the authoring pipeline and variety for a million (seeds, face archetypes); clothing in layers by offline binding and morphs; animation (locomotion, smart objects, walking together, F-formations, first-person body, budget); staged dialogue, barks, a line-id format, hooks for language models, visemes later; the in-world device; relationships and memory; netcode by parameters; patents (EA, Nintendo, Tencent, JALI, NVIDIA, Meta, Warner Bros.' Nemesis, Sony, Roblox) | 42 | done (2026-10-10: the owner's showcase brief; sources read or search-recorded, graded per entry) |
| [research/transit-and-vehicles.md](research/transit-and-vehicles.md) | transit and vehicles: trams and boats as timetabled runs with delays (GTFS's model, MATSim's and SUMO's transit), headway holding and dwell caps; the clock's arithmetic (four game seconds a real second quarters a network's capacity per game hour); public automated taxis as a dispatcher (insertion, DRT, rebalancing between stands, quoted waits, Little's law, the wild goose chase), taxis on tram track only in booked gaps; river boats as frequency shuttles that dock themselves; walking and cycling shares and mode choice; vehicles near players (kinematic trams on bogie points, IDM taxis, Jolt boats); the player riding and piloting in the vehicle's frame; RAPTOR for the smart pad and live displays; later ships, aircraft and drones; patents (pooling and dispatch, signal priority; lapsed: Waymo's fly-by pick-up, Microsoft's RAPTOR) | 61 | done (2026-10-10: the owner's showcase brief; sources read or search-recorded, graded per entry) |

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

**Planetary environment.** A game climate is twelve months of a few fields and a lookup, with
Köppen to validate and BIOME1-style plant tolerances to pick biomes. Every generator that looks
right derives the fields from latitude, altitude and wind-carried moisture with rain shadows
(mapgen4, Dwarf Fortress, AutoBiomes, Houdini 20.5's biome tools), not from noise. The physics is
cheap enough to bake: insolation by obliquity, a Budyko–North energy balance, and Smith–Barstad
orographic rain per terrain tile, with ExoPlaSim run offline as the oracle. Ecotones are soft
where only the climate changes, and sharp where a feedback switches the ground (fire, water,
shade), so biome weights are a scattered-kernel blend in climate space sharpened by local rules.
Weather rendering is settled 2004–2013 technique: Garg–Nayar streaks, Lagarde's wet surfaces,
depth-from-above occlusion for rain and snow, Nubis weather maps. Simulated weather
(Stormscapes → Cyclogenesis) is interactive only over tens of kilometres. D-019's struct becomes a
sample of three layers (D-034):
- a baked climate atlas (≈ 78 km, 19 MB);
- weather as a pure function of atlas, seed and time, so only the clock and events are
  replicated;
- wetness, puddles and snow integrated in a clipmap around cameras.

**Data-driven engine and mods.** Every engine that serves many games puts a reflected type system
between code and content and references content by stable id, not by path (Unreal's Primary Asset
Ids, Unity's asset IDs, Godot's `uid://`, completed only in 4.4). Every moddable game layers data
with a load order and a conflict rule: Bethesda's rule of one, Paradox's LIOS, Minecraft data packs,
RimWorld's XPath patches, Factorio's three data rounds sorted by dependency depth. D-007's table is
already that pattern. What Forge needs (D-035):
- `package:path` ids, with dense per-table indices from the sorted set;
- RON records with schema versions;
- packages merged by add/replace/patch, with a conflict report;
- a manifest hash in D-016's digests and D-010's handshake.

Code stays statically linked Rust, since there is no stable ABI; `subsecond` hot-patches systems in
development. Mod code, if wanted, runs as Wasm components under wasmtime, with its deterministic
settings and fuel, server-side by default. Wasm 3.0 specifies a deterministic profile, whereas
Factorio had to replace Lua's maths and iteration order to keep lockstep. The editor is the long pole
(Bevy still has none) and can wait; the data model is what to decide now.

**City generation.** Streets are solved twice: Parish & Müller's propose-validate-commit loop
for the hierarchy and Chen et al.'s tensor fields for the pattern, both seconds of CPU for a 4 km
city, with Emilien's interest maps and Galin's cost paths for a village; lots are Vanegas et al.
2012 (oriented-box splits for the grid, straight-skeleton strips for the old town); districts a
land-use field, landmarks authored overrides merged as layers, judged by Lynch's five elements.
Research grammars (Instant Architecture, CGA, CGA++) make unique buildings; every shipped city
(Skyrim, The Division, Watch Dogs: Legion, Epic's City Sample: 24 kits, 7 000 buildings of hundreds
of instances) assembles kits. Forge's proposal (D-039 ✅): a CGA-style split grammar whose
terminals are kit modules on a 0.5 m grid, instanced through the existing cluster-DAG path, a
style set per district with its regional trim sheets, a merged proxy per far building, rooms and
portals as records for the interiors; the kits themselves generated at cook time, since there is
no art team. A `CityPlan` of typed records (roads, blocks, lots, districts, landmarks, buildings as
module instances) is the point cloud the City Sample's rules processor consumes.
*Extended 2026-10-10 (§7–§19):* a green, transit-first European city by the sea needs no new kind of
generator. It needs:
- more records in the `CityPlan`: the coast and river taken from elevation and hydrography data,
  green, transit lines and stops, and uses;
- one style-bible file that every stage reads and checks itself against: heights by street width,
  cornice lines, the rhythm of openings, the 3-30-300 tree rule, Berlin's green-area factor;
- street profiles with vehicle filters by class and hour, as Ghent and Pontevedra do;
- interiors in tiers that copy a shipped method (Shadows of Doubt's scored greedy room placement
  on D-039's bay grid, with partition walls only on the piers between windows), not a research
  solver.

Learned floor-plan generators are research-only by licence. A city of a million has about half a
million dwellings and 20–30 thousand shopfronts, so interiors come from seeds on demand. A tram
line carries 3 000–11 000 people an hour each way, so a dense city of a million needs many lines or
an underground trunk.

**HDR display output.** On Windows the game presents 10-bit PQ over Rec.2020 or linear fp16 scRGB
(1.0 = 80 nits), the compositor converts both, and the one OS value to read is the user's SDR white
level, or UI and desktop-like content look dim (Microsoft's Advanced Color guide). Vulkan lists the
same two pairs on NVIDIA and AMD through `VK_EXT_swapchain_colorspace`, which Forge's instance does
not enable yet, leaves the PQ encoding to the shader, and carries mastering metadata through
`VK_EXT_hdr_metadata`, which the presentation engine may ignore; `ash` 0.38 has both. ACES 2.0's
output transform is the first standard curve with HDR presets (500/1000/2000/4000 nits, P3-D65 or
Rec.2020 limiting, PQ), OpenColorIO 2.4–2.5 implements them as builtins, and Forge's port already
passes OCIO's 1000-nit P3 test values, so HDR is a rebake of the 65³ table with the shaper's top at
the transform's own clamp (4096 at 1000 nits) and PQ-encoded 16-bit UNORM texels. The look decision:
the presets put a scene white at 107 nits at 1000 nits (grey 14.5), below Windows' desktop white
and BT.2408's 203 nits, and every shipped engine (Frostbite, Unreal, Unity, Call of Duty, Sucker
Punch) exposes a paper-white control and draws the UI at it, so Forge needs a paper-white gain in
front of the transform and the overlay written at that white, plus a three-pattern calibration
overlay (HGiG's MinTML/MaxTML/MaxFFTML, Windows' own app). Ten-bit PQ steps are about 1 % of
luminance, so the display pass dithers half a code. Everything but the present is verifiable on an
SDR monitor through an offscreen 10-bit target and a "fake HDR" preview; the cost is a few
thousandths of a millisecond.

**Render graph, next (#78).** Transient buffers are the lifetime analysis and the first-fit heap
D-020 already runs for images, extended to buffers the way Unreal's transient allocator does it and
Frostbite did not (an atomic linear allocator for buffers) and Granite still does not ("Buffers are
never transient"). Vulkan adds three rules: a buffer next to an image in the heap is padded to
`bufferImageGranularity`, any overlapping use with a write needs a memory dependency (the graph's
first-use barrier already is one), and the contents after the other alias writes are undefined,
which makes a stale device address the one new bug, and one that neither synchronization validation
("hazards related to memory aliasing are not detected properly") nor GPU-assisted validation (an
aliased buffer is a valid address range) can see, so Forge writes a poison mode and hands out
addresses only inside pass bodies. The packing is dynamic storage allocation, NP-complete, with
LOAD (the peak of live bytes) as the bound to print next to the heap size; a smarter order is worth
about 11 % at most. Parallel recording has one published shape (Granite's task per physical pass is
the closest code): pools per worker per frame in flight, the plan frozen before the jobs start,
each job recording a chunk of consecutive passes with its barriers into its own primary command
buffer, the buffers of a batch submitted in order in one `vkQueueSubmit2`; NVIDIA and AMD both
warn that submissions cost a kernel call and every command buffer carries driver work, and a 2026
measurement on an RTX 5070 puts fifteen redundant barriers at 29 % of GPU span, so chunks, not
passes, and no extra barriers. Events split a barrier but cannot cross queues; #77's timelines stay.
Forge's recording is 0.08–0.10 ms of an 8.33 ms budget today, so the order is: transient buffers
and the poison mode, the meshlet lists migrated one by one at 0 px, the CPU zones split in the
overlay, a synthetic 100-pass frame to size the gate, and parallel recording only once the bodies
pass 0.5 ms or a frame passes 100 passes.

**Planet terrain.** The partition is settled and in the code: D-037's equi-angular cube sphere
with S2-style ids is what every planet renderer converged on (Cesium's geographic quadtree pays
at the poles, HEALPix's equal areas come on rhombi). The heightfield LOD literature offers three
ways of avoiding a mesh hierarchy — spherical and ellipsoidal clipmaps, CDLOD's morph, a
longest-edge bisection kept on the GPU in a concurrent binary tree ("planetary scale geometry out
of very coarse meshes" in under 0.2 ms) — and Forge already has the hierarchy, as Epic's landscape
has since 5.3, so a planet's tiles are cluster-DAG props like the city's ground: one per cell of
D-037's clipmap, 257² samples plus a halo of 8 for matching normals, cooked from a field the
genesis amplifies at streaming time in `Low` jobs from a coarse global graph (six faces of 1025²,
about 20 s), locked at their borders so equal levels never crack, a skirt for the level change,
the six level-0 tiles always resident, the pages streamed by the existing pool. The horizon is
Cesium's two-dot-product test on one occlusion point per tile in the instance cull; precision and
depth are D-004 as they stand; the atmosphere from space is D-023 with the terrain's depth as the
march's floor; the shadows a BLAS per tile from its coarse cut. Every shipped planet (Elite, No
Man's Sky, Star Citizen's fifth planet tech, Space Engineers, KSP2) generates tiles at run time
around a quadtree, and the ones that look right at walking distance are adding a global
simulation behind them, which the coarse genesis is. "No pop" gets a definition: a finer tile
replaces its parent only when the parent's clusters there are under a pixel of error, and the
LDR-ꟻLIP between the frames before and after every swap of a scripted descent stays under
D-017's 0.15 peak. Orders of magnitude: a tile is 131 k triangles, about 2 100 clusters and 27
pages, a quarter of a second to produce and cook, 3–4 wanted a second at 300 m/s, 200 resident, a
128–256 MiB pool, 1–2 ms of terrain at 1440p at every altitude, and a digest per tile's field
checked at one and six workers. The CBT is the spike to run against it afterwards, with the same
field as a texture; the island is placed on the planet as an uplift override of the coarse graph
so the far tiles already show its shape.

**Civilisations and styles.** Why buildings look the way they do is already written as
parameters: Rapoport puts culture first with climate, materials and technology as modifiers,
Oliver's encyclopedia catalogues the result by region, and the bioclimatic school (Olgyay's four
regions, Givoni's chart, the Mahoney tables) turns twelve months of temperature and humidity into
orientation, wall mass, opening ratio, roof and courtyard-or-stilts — a hundred-line function of
D-034's atlas cell. A style is a grammar with attributes, fifty years over: Palladio, Wright, the
Queen Anne house, Siza's Malagueira (1 200 houses from a corpus of thirty-five), the Yingzao Fashi's
rank-to-module system, Knight's tatami grid, Kaplan's star patterns, Havemann's Gothic tracery, CGA
on Pompeii and the Puuc Maya; each contributes a column (plan rule, grid, bands, openings, roof,
articulation, ornament, detail field, era) of D-039's control grammar, so a culture and an era are
data over one split grammar. Decor is generators plus solvers (Infinigen Indoors' 79 BSD generators
and constraint DSL, ShapeAssembly's cuboid programs, Merrell's and Yu's layout terms, Fisher's
clutter, Germer's dress-when-streamed rooms); materials are the canon plus recursive stamps and
phasor noise for masonry and weave, by-example synthesis cooked once, and ageing by flow, patina,
slab erosion, γ-ton exposure and appearance strips, with the neural material work reference only
(P3/P8, D-016). Every shipped game hand-sets a few architecture sets per culture group (Age of
Empires II's eleven-plus, Age of Empires IV's four ages per civilisation, Minecraft's five biome
pools, RimWorld's five styles, No Man's Sky's six archetypes at seven-to-three per race) and
generates only within a set, as data where the game is data-driven; none generates the set, which
is Forge's gap and Forge's sameness risk. The proposal: a `Civilisation` record as a D-035 layer
that selects and patches D-039's style sets per building class, with resources → palette, class
frequencies, permission lists, motifs, a wear profile and a climate response derived from the
atlas; the 0.5 m grid from a hut to a habitat's curved floor, with tents, organic forms and
megastructures on their own paths; textures cooked from generator graphs to KTX2 by content hash
(about 100–150 MiB and 10–20 s per style set, estimates); and a build order that starts with the
response function and the record over the two existing sets, then the material generator, wear, a
primitive set, a futuristic set for the belt's bases, props and dressing, and only then a third and
fourth culture to measure how much of a culture is data.

**Game AI and ecosystems.** Behaviour selection has four published shapes and the owner has
picked two of them for the villagers and the animals (utility scoring over needs inside a schedule
of packages re-evaluated by condition — The Sims' advertisements and Boltzmann pick, ArenaNet's
Infinite Axis Utility System, Oblivion's and Skyrim's package stacks — with a small behaviour tree
for interrupts in Isla's Halo 2 shape) and a planner (F.E.A.R.'s GOAP, Humphreys' total-order HTN
if it outgrows it) for the reconstruction jobs alone; smart objects carry the affordance, the
reservation and the animation, so a new prop is data. Navigation is Recast's voxel pipeline on
tiles the terrain edits invalidate (Mononen's tile cache rebuilt one tile in about 2 ms in 2011),
Polyanya's any-angle query with layer flags for doors and fire, flow fields per attractor for the
market (Supreme Commander 2, Planet Coaster's 10 000 guests), ORCA for the last metre, Reynolds'
steering under everything, a sparse voxel octree with Theta* for birds, fish and ships, and the
road polylines as a lane graph for the traffic, never the navmesh. Every living world that works
is an AI level of detail — Assassin's Creed Unity's 40 real AIs and 120 bodies inside 10 000,
Stalker's A-Life offline graph and Stalker 2's 2024 failure when its radius and memory were cut,
Watch Dogs: Legion's population as database rows — and that is the planet's shape too: a
statistical layer per climate cell in game-day steps, a regional layer of records on the cell
graph out to kilometres, a full layer within a few hundred metres, with hand-offs that are pure
functions of the seed (spawn at need zones, fold deaths and births back into densities) and
Chenney's consistency and completeness as the tests. Animals are the same machinery with needs,
per-species senses, a herd as a group agent with Couzin's zones and Horizon's role split, a home
range for predators, and populations calibrated against NetLogo's Wolf Sheep Predation and written
in Grimm's ODD form; ships are Elite's page of rules and FreeSpace's goal stack as utility actions
over thrust-limited steering, a threat table, formation slots (Balch & Arkin) and a PID autopilot,
with Isaacs' barrier deciding turn-in against run and convex descent guidance kept for a landing
that may never come; learned policies (GT Sophy, AlphaStar, the AlphaDogfight hierarchy) win but are
not a function of a seed, so they stay off the tick. In Rust the geometry exists to read and to
diff against (`rerecast`, the deterministic fork behind `recastnavigation-rs`, `polyanya`, `dodgy`,
`landmass`), the decision crates are the right shape but not the right contract (`big-brain` three
Bevy versions behind, `bevy_behave`'s entity-per-task), so Forge writes the generator, the query,
ORCA, the scorer, the schedules, the smart objects, the LOD and the digests, and depends on
`pathfinding` and a small tree crate. The build order starts with `forge-sim`'s tick and digest,
then the navmesh over the island rebuilt around a crater, twenty villagers with schedules and smart
objects, two animal archetypes and a herd, the LOD with its radius test, the crowd and the carts,
the reconstruction planner, the ships for the ballad, and only then the planet's layers; the budget
to measure is about 20–50 full agents per millisecond per core, thousands of regional records, and
a few hundred nanoseconds per species per cell per game-day, all estimates until the overlay says
otherwise.

**Smart systems.** What keeps shipped games fluid is heuristics with budgets, not trained
models: need measured every frame from the view (screen-space error, distance, screen size,
visibility, the last frames' GPU time), prediction that is short-range and kinematic or scripted
(streaming sources weighted by heading and velocity, known cameras and transitions, I/O queues with
priorities), and controllers with hysteresis holding budgets (Unreal's animation budget, dynamic
resolution, population caps with spawns out of sight, per-client byte budgets filled by priority).
Nothing public describes a GTA VI system that "optimises every scene"; what is on record for
Rockstar is GTA V's separate budgets for distance, population and variety, and less streaming
detail while the player flies. Learned models win where their errors show only as pixels
(upscaling, frame generation, denoising, radiance caches, texture compression, deformers) or where
a decision is made offline or on a server with a person in the loop (VACnet, test bots); learned
movement prediction has not beaten trajectory extrapolation at the few seconds streaming needs.
Shader inference in Vulkan is still vendor-split, and float networks are not bit-reproducible, so
what client and server must agree on stays heuristic, server-only or integer. For Forge now: a
free-flight lead and a speed cap for the planet's tile cut, two classes of page request, sliced
edits and a spike recorder to name its worst frames; later a significance service with budgets in
`forge-sim`, an optional frame-time governor, and test bots before any learned predictor.

**A galaxy's sky.** An eye sees about 9 100 stars to magnitude 6.5 on a dark night on Earth, and
about 8 with no air (about 42 000 stars). The limit follows the eye's adaptation, which in a
renderer is the exposure: 0.753 magnitude a stop. The limit does not make the sky a neighbourhood:
one naked-eye star in seven is over 1 000 light-years away. What it allows is a tree of cells sorted
by brightness, each level 1.505 magnitudes fainter in cells half the size, so a visit costs a
constant number of cells a level. Celestia stores its stars that way, SpaceEngine too, and
Elite Dangerous as far as players have reverse-engineered it. A toy galaxy gives the sky to 6.5 from
any place in about 600 cells and 10⁵ generated stars, milliseconds once per jump, whatever the
galaxy's size. A star's id is its level, cell and index, so a point of light names a system to jump
to. One sky per system is exact to half a pixel. The light fainter than the limit (the Milky Way
band) and the dust belong in a coarse volume, which is what voxels are for. The eye, a bridge
telescope and the galaxy map are the same query with a field, a limit and a centre. Nothing is
needed before the galaxy phase; its decision comes after D-057.

**Assets and licences.** Free, redistributable assets are enough for surfaces, props and
placeholders, but not for release-quality people, buildings and vehicles. Poly Haven and ambientCG
(CC0 scans), Kenney and KayKit (CC0 low-poly), MPFB's assets (CC0, some CC-BY), NAVER's Anny (Apache
code over CC0 data) and Microsoft's Rocketbox (MIT) may be fetched by a public repository. Quaternius
moved to a no-redistribution licence on 28 August 2026. Everything at AAA quality can ship in a
game but may not be redistributed: MetaHuman (any engine since June 2025, free below $1 M; its DNA
and RigLogic runtime MIT since June 2026), Fab and Megascans, Reallusion, Daz, KitBash3D,
TurboSquid, CGTrader, Substance assets, trees from SpeedTree or The Grove (SpeedTree's rights end
with the subscription). Most of these licences forbid AI training and ask for extraction-resistant
packing in a release. Fab and Textures.com also forbid mixing their content into open-source
licences. The SMPL family is non-commercial. Free and paid sources alike lack a realistic building
kit with interiors, futuristic vehicles in one style, and park trees, so those are made, or grown
with a tree tool.

**A persistent population.** At any moment nearly everyone is inside a place or on a known route
begun at a known time. So a million people is a data problem, not an AI problem. Kept as records of
about 100–130 bytes (0.15–0.2 GB at a million with the routes and graphs) and advanced by events
rather than ticks, the whole population costs a few percent of a core. Even MATSim's queue
simulation, scaled to a clock four times real time, puts it at about 1.4 cores at four million. The
costly people are the hundred or two near each player, which every shipped game caps (Assassin's
Creed Unity's 40 full AIs, Vuntra City's roughly 100 spawned out of four million). The seams hold
when every tier answers "where is this person now" from the same record. The server's CPU decides
everything (D-016), the GPU generates offline and draws the crowd on the client, and the near people
cost 10–20 KB/s per client. Improbable's interest and hand-off patents (to 2036–2037) and Take-Two's
coarse-graph routing patent (to 2039) each have an alternative that was checked against their
titles and summaries; their claims were not read in full.

**Procedural content.** Procedural methods take a small team from blockout to a consistent AA look
for everything man-made, and to AAA for the materials of man-made surfaces. They fall short where
the eye reads nature or authorship. Forge's terrain and water generators are research-grade. Its
objects are displaced boxes and bevelled primitives in flat colours, and its twelve textures are
albedo and normal at 512². No algorithm is missing. Three layers every studio pipeline has are
missing:
1. full PBR materials with wear derived from the geometry, plus trims and decals (Material Maker,
   MIT, scriptable from the command line);
2. objects as part programs (Infinigen, BSD-3, is the largest open corpus of such generators);
3. a cook stage that cleans, unwraps and bakes into the D-053 cache.

Scans stay for bark, leaves, ground and stone; hand art for landmarks, signage, hero vehicles and
faces. Generative 3D AI is not an engine path: it breaks D-016 and gives triangle soup with baked
shading, and Hunyuan3D's licence excludes the EU. Live patents sit on Adobe's Substance engine,
particle-trace texturing and PatchMatch method, so Forge cooks offline and derives wear from
geometry. Image quilting, shape-grammar interpreters and marching cubes have expired.

**People.** The people a player looks at and the people a player sees are two systems joined by one
identity.
- **Up close:** a MetaHuman-class asset (a DNA file evaluated by RigLogic, MIT since 2026; up to 713
  face joints and 669 blend shapes at the top LOD), with screen-space subsurface skin, a refractive
  eye, hair cards and fitted clothes.
- **At a distance:** beyond about 15 m a face is under 20 pixels, and beyond about 150 m a whole
  person is under 15. So:
  - a skeletal path for the ~150 people near a player;
  - baked bone-animation-texture meshes with pre-posed shadows for the next few thousand;
  - impostor cards out to 1.5 km.

  Unreal 5.8's MetaHuman Crowds makes the same swap.
- **Variety:** a million people need a million seeds, a few dozen face archetypes, a few hundred
  garments and a colour system.
- **Clothes:** bound to the body offline with per-garment morphs, which steers clear of Roblox's
  cage and EA's deformation-map patents.
- **Dialogue:** staged like The Witcher 3's systemic dialogues and carried as line ids, with
  language models later as a recorded input.
- **Relationships:** a few bytes per pair that ever met.
- **Patents:** Warner Bros.' Nemesis family (to 2036) is the one to discuss before building gossip
  or memories that spread between people.

**Transit and vehicles.** The transit layer is a timetable with delays, not a traffic
simulation. Trams and boats are timetabled runs whose position is a pure function of their
schedule, delay and speed profile, so the server, every client, every stop display and the pad
compute the same tram from a few bytes. People board and alight as events. The clock is the
hardest number: at four game seconds per real second, with vehicles at real speeds, a network or
fleet copied from a real city carries about a quarter of its trips per game hour, unless trips
are four times shorter.
- **Public automated taxis** are a dispatcher: insertion in batches from fixed stands, idle
  vehicles rebalanced between stands, a quoted wait that mode choice reads. They enter tram track
  only in a booked gap and never stop on it.
- **Boats** are frequency shuttles that dock themselves.
- **Walking and cycling** carry most trips in every transit-first city measured.
- **Near players:** trams are kinematic on bogie points along the track, taxis are kinematic with
  the Intelligent Driver Model, boats are Jolt bodies on the existing buoyancy and currents. The
  player rides in the vehicle's frame.
- **The pad** routes with RAPTOR (Microsoft's patent on it has lapsed).
- **Patents:** pooling and dispatch (to 2035–2041) and signal-priority timing (to 2037) are
  live, each with an open alternative.

## Still to research

- Tools and editor: creation graphs and the editor itself. The data model and hot reload are
  covered in data-driven.md (D-035); the previous bibliography's §6 covers the rest, and a
  Forge-specific file comes with the editor.
