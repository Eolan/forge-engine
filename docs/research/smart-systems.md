# Research — Smart systems: prediction, importance and learned models

> Companion to `memory-streaming.md` (§4: need-driven residency, Nanite, Ghost of Tsushima),
> `large-worlds.md` (§5 World Partition, §6 the Significance Manager and Mass), `netcode.md` (§1
> the priority accumulator, §3 the Replication Graph and server meshing), `game-ai-ecosystems.md`
> (§3 the simulation LOD, the AI Director, learned policies), `lighting-gi.md` (§1 neural caches,
> §9 upscalers), `animation.md` (§2 learned motion) and `planet-terrain.md` (§4 the speed cap and
> the velocity prefetch). Written 2026-10-10 for the owner's question of that day, against D-005,
> D-010, D-012, D-016, D-018, D-025, D-053 and D-056 as they stand, `docs/demos/planet.md` and
> `docs/PROFILE.md`. Sources were checked with WebFetch and WebSearch only, no browser pane, and are
> paraphrased; each one's grade (page read, or search record only) is under
> [Verification notes](#verification-notes).

The owner asked whether Forge uses "heuristics or smart systems" as engines do: to load what the
current context will most likely need, to decide what is drawn precisely and what coarsely, and to
keep every scene fluid; and whether trained models could serve, on the client and on the server of
a networked game. The short answer: what keeps shipped games fluid is almost entirely heuristics
with budgets, evaluated every frame on cheap signals (screen-space error, distance, visibility,
velocity, measured milliseconds), plus scripted hints where the game knows its own future. Trained
models win where their output is pixels, or a decision made offline or on a server; they have not
yet beaten simple extrapolation at guessing where a player goes in the next few seconds. Forge
already spends its geometry by need. What it lacks is prediction in free flight, a guard against
its worst frames, and, later, one budget service for everything that is not geometry.

> **State of the art in five sentences.** Engines spend their budget by need measured each frame
> from the view (screen-space error for geometry and mips; distance, screen size and visibility for
> animation, AI and replication; the last frames' GPU time for resolution and shading rate), with
> recency only to break ties. Prediction in shipped engines is short-range and kinematic or
> scripted: streaming sources weighted by heading and velocity (Unreal, Roblox), known future
> cameras and transitions (cinematic prestreaming, Ghost of Tsushima's camera cuts, Insomniac's
> rifts), and I/O queues with priorities (DirectStorage's weighted round-robin, the PS5's six
> levels). Budgets are held by controllers with hysteresis, not by search: an animation budget in
> milliseconds that lowers the tick rate of the least significant meshes, a dynamic resolution that
> panics after consecutive over-budget frames, population caps with spawns placed out of sight, and
> per-client byte budgets filled by accumulated priority. Trained models have won where errors are
> soft and show only as pixels (upscaling, frame generation, denoising, radiance caches, texture
> compression, deformers) and where the decision is offline or server-side with a person or an A/B
> test in the loop (VACnet, EA's difficulty service, test bots), while learned movement prediction
> has not beaten trajectory extrapolation at the sub-3-second horizons streaming needs. In October
> 2026 shader inference is still vendor-split in Vulkan (cooperative vectors NVIDIA-only,
> cooperative matrices cross-vendor; DirectX folds both into a Linear Algebra API in preview), and
> float networks are not bit-reproducible across platforms, so whatever client and server must agree
> on stays heuristic, server-only, or integer.

---

## 1. Predictive streaming and prefetching

**Epic Games. "World Partition", `FWorldPartitionStreamingSource`, Cinematic Prestreaming
(`UCinePrestreamingGraphNode`). Unreal Engine 5.8 documentation.** [docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/world-partition-in-unreal-engine> ·
<https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Engine/FWorldPartitionStreamingSource> ·
<https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/CinematicPrestreamingEditor/UCinePrestreamingGraphNode>

A streaming source (the player controller, or any component) loads the grid cells around it with a
priority (a cell takes the highest of the sources touching it), a shape and a target state. A cell's
importance also weighs the angle between the source's forward vector and the cell. The source
record carries a velocity computed from its position history and a flag that lets it weigh in the
sorting of cells; the weighting is not documented. A grid can block the game when cells load too
slowly, and concurrent cell loads are capped. For scripted sequences, Cinematic Prestreaming records
the virtual-texture and Nanite page requests of an offline render into an asset that the runtime
replays ahead of playback; recordings depend on the output resolution.
*Bearing:* the dominant engine predicts by dead reckoning on a heading and by recording the real
requests along a known path. Forge has both halves: the planet tour is a known path, and the culls
already write the requests a recording would keep.

**Mike Fitzgerald (Insomniac), PlayStation Blog interview, 6 September 2018; Sony, "How PS5 helped
make Ratchet & Clank: Rift Apart possible".** [web]
<https://blog.playstation.com/?p=204132> ·
<https://www.playstation.com/en-hk/editorial/how-ps5-helped-make-ratchet-and-clank-rift-apart-possible/>

Spider-Man's Manhattan is about 800 tiles of 128 m; the streamer drops tiles behind the player and
loads tiles ahead, about one a second at top swing speed. Rift Apart swaps whole worlds in seconds,
and memory holds what the moment needs. The rifts are placed by designers, so the next world is
known before the jump (a rival developer argued in the press that two buffers and a short void
would do it on old hardware; search record): the hint is the design. The GDC 2019 Spider-Man
post-mortem (`memory-streaming.md` §4) is login-gated.
*Bearing:* a fast-traversal game streams a ring ahead at a rate set by top speed. One tile a second
is the scale Forge's planet already meets: an edit of up to ten tiles is ready 11 ms after it is
asked for.

**Roblox. "Instance streaming", `PredictiveStreamingMode`. Creator documentation, 2026.** [docs]
<https://create.roblox.com/docs/en-us/workspace/streaming>

The server streams to each client around replication foci: inside a minimum radius at the highest
priority and never streamed out, more up to a target radius, the farthest regions dropped first.
The predictive mode adds temporary foci (respawn points while a player is dead, regions just left by
a teleport); predictions expire if unused and are skipped on constrained devices.
*Bearing:* the clean form of a prediction that cannot hurt: additive, short-lived, ranked below
what is seen, dropped first under pressure.

**Ghost of Tsushima, Horizon Zero Dawn, Flight Simulator** are in `memory-streaming.md` §4 and
`planet-terrain.md` §4: Sucker Punch prefetches for the next cutscene and every camera cut and
squeezes its texture budget by a distance penalty; Guerrilla streams per domain with designer
programs; Asobo fetches tiles per level and layer. No statement on Flight Simulator 2024 prefetching
along a flight plan was found.

**I/O priorities.** DirectStorage queues take one of four priorities, enforced by a weighted
round-robin on request sizes, adjacent levels about an order of magnitude apart (Microsoft GDK,
`DSTORAGE_PRIORITY`, <https://learn.microsoft.com/en-in/gaming/gdk/docs/reference/system/dstorage/enums/dstorage_priority>).
Cerny said NVMe has two true priority levels and the PS5's controller six (2020, press; search
record).
*Bearing:* a prefetch is safe only in a lower class than demand: Forge's I/O thread (D-018) needs a
demand queue and a speculative queue, not one sorted list.

**Learned prediction of access and movement.** [paper]
- **Milad Hashemi et al., "Learning Memory Access Patterns", ICML 2018 (arXiv 1803.02329).**
  Recurrent networks beat table prefetchers in precision and recall on traces, offline; the authors
  call it a first step. No game streamer using such a model was found.
- **Miguel Fabián Romero Rondón, Lucile Sassatelli et al., "TRACK" (arXiv 1911.11702; IEEE TPAMI).**
  On the public 360° video datasets, every published deep head-motion predictor that also used the
  video did worse than baselines using the past trajectory alone; content helps only beyond 2–3 s.
- **Kyungmin Lee, David Chu et al., "Outatime", MobiSys 2015 (Microsoft Research).** Cloud gaming
  that renders speculative frames of likely futures one round trip ahead and rolls back on a miss,
  hiding up to 120 ms in Doom 3 and Fable 3.
- **Tristan Walker, "Dead Reckoning for Distributed Network Online Games", Waterloo, 2021.** A
  network predicted opponents' positions better than classic dead reckoning, but path tracking gave
  smoother trajectories.

*Bearing:* at streaming's horizons (Forge's tile cut looks 0.25–4 s ahead) inertia dominates:
extrapolate the trajectory and use scripted knowledge. A learned predictor earns a trial only once
logs show where those miss.

**Forge today** (`docs/demos/planet.md`, `crates/forge-render/src/streaming.rs`). The planet's
worker remakes the cut every 50 ms around where the camera will be once a change is ready (the last
change's latency ahead, clamped to 0.25–4 s) and around where the run heads. That lead exists only
on the scripted tour and descent: in free flight the cut follows the current camera, and the speed
cap `planet-terrain.md` §4 proposed (`speed × latency < reach`) is not built. Cluster pages are
requested by the culls for instances inside the frustum, by the pixels of error each page removes;
a page no frame wants has need zero and goes first, the least recently wanted among equals. The CPU
can already work out a view's pages (the start view, #121). D-018's "camera prefetch" request source
is not built.

---

## 2. Importance: what to draw precisely

**Screen-space error** is settled: Nanite's cut and page priority are in `memory-streaming.md` §4;
Forge's culls use a pixel threshold, and the planet's swap rule splits a cell where its samples would
stand over 2.5 px apart on rough ground or its height would show over a pixel. Engines differ in how
the threshold moves under load.

**Epic Games. "Dynamic Resolution." Unreal Engine 5.8.** [docs]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/dynamic-resolution-in-unreal-engine>
The screen percentage follows the previous frames' GPU time against a budget (33.3 ms by default):
a headroom percentage, a minimum change, a minimum number of frames between changes, bounds (50–100 %
by default). After a set number of consecutive over-budget frames it panics: drops at once, clears
its history, climbs back slowly. A separate target applies when the CPU is the bottleneck.
*Bearing:* the full shape of a frame-time governor: target with headroom, hysteresis, a panic path,
a history whose length trades noise for lag.

**Jorge Jimenez (Activision). "Dynamic Temporal Antialiasing in Call of Duty: Infinite Warfare."
SIGGRAPH 2017 Advances.** [talk] <https://advances.realtimerendering.com/s2017/index.html>
Under load the temporal reconstruction loses quality while the output keeps full resolution, and
shader and model LOD absorb spikes. *Bearing:* with TAA or DLSS, the input size is the cheap knob.

**Alex Vlachos (Valve). "Advanced VR Rendering Performance." GDC 2016.** [talk]
<https://gdcvault.com/play/1023522/Advanced-VR-> (the Lab renderer, BSD-3:
<https://github.com/ValveSoftware/the_lab_renderer>)
Adaptive fidelity held 90 fps without reprojection: a demo that needed a GTX 980 ran on a GTX 680.
The thresholds are in the slides, not reached; the renderer ships the system as source.
*Bearing:* a governor lowers the minimum GPU rather than the look.

**Variable rate shading. Jacques van Rhyn, Chris Wallis (The Coalition), "Moving Gears to Tier 2
Variable Rate Shading", DirectX blog, 12 January 2021; Michal Drobot (Infinity Ward),
"Software-Based Variable Rate Shading in Call of Duty: Modern Warfare", SIGGRAPH 2020 Advances.**
<https://devblogs.microsoft.com/directx/gears-vrs-tier2/> ·
<https://advances.realtimerendering.com/s2020/index.html>
Gears derives a shading-rate image from a Sobel filter on the previous frame's luminance, no motion
term, in under 0.1 ms; on an RX 6900 XT at 4K it saved 8–12 % at Ultra and 14–20 % with screen-space
GI, with little gain at 1080p and below. Drobot does it in software for compute passes, on any GPU.
*Bearing:* Forge shades in compute after the visibility buffer, so only the software route applies,
per 8×8 tile in `shading/classify`; at 0.3 ms of `shading/layered` it can wait.

**Foveation** (Unity, 22 February 2023, <https://unity.com/blog/games/the-next-generation-of-vr-gaming-on-ps5>):
foveated rendering on PS VR2 up to 2.5× faster, 3.6× with eye tracking. Gaze is a strong importance
signal; a monitor game lacks it, and no shipped model that guesses it (saliency) was found.

**Feedback and occlusion.** Sampler feedback and depth-tested VT feedback are in
`memory-streaming.md` §4: hidden content asks for nothing. Forge's culls write needs after the
instance's frustum test but before the cluster's frustum and occlusion tests, so a hidden cluster of
a visible instance keeps its page.

---

## 3. Importance: what to update and simulate

**Epic Games. "Animation Budget Allocator", `FAnimUpdateRateParameters`, "Overview of Mass
Gameplay." Unreal Engine 5.8.** [docs]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/animation-budget-allocator-in-unreal-engine> ·
<https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Engine/FAnimUpdateRateParameters> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-mass-gameplay-in-unreal-engine>

The allocator caps skeletal-mesh work at a game-thread budget (1.0 ms by default, raised per
platform). Over budget, the least significant meshes tick every N frames, interpolate, or stop; the
most significant keep full rate. Its controls show how a budget stays stable: pressure thresholds at
1.5, 2 and 2.5 times the budget, smoothing of the measured cost, a cap on off-screen meshes ticked,
and throttles on how often a mesh may change state. The older update-rate optimisation picks a frame
skip from screen size and evaluates non-rendered meshes (off screen, dedicated servers) at their
own rate. Mass computes one LOD per entity (High, Medium, Low, Off, with distances and maximum
counts per level, and a continuous significance from 0 to 3) read by representation (with distances
that differ in and out of the frustum), simulation (a variable tick) and replication (per client).
The Significance Manager is in `large-worlds.md` §6.
*Bearing:* the shape for Forge's later budget service: one significance per entity, read by every
system, each with a millisecond budget and anti-flicker throttles.

**Unity. "Adaptive Performance: scalers." Unity 6.6.** [docs]
<https://docs.unity3d.com/Manual/adaptive-performance/scalers-introduction.html>
An indexer turns thermal and performance state into a quality index; scalers (LOD bias, render
scale, frame rate and others) step down for the current bottleneck first and the lowest visual
impact first. *Bearing:* "cheapest visible loss first" as an explicit order.

**Populations and traffic.**
- **Michael Booth (Valve), "The AI Systems of Left 4 Dead", AIIDE 2009 slides**
  (<https://steamcdn-a.akamaihd.net/apps/valve/2009/ai_systems_of_l4d_mike_booth.pdf>). The
  Director populates an active area set of navigation areas around the team, creating and
  destroying enemies as it moves, so hundreds of enemies come from a few reused entities. Spawns go
  where no survivor can see, three quarters of mobs behind the group by travel distance. Pacing
  tracks each survivor's intensity (damage, incapacitation, nearby kills), decays it, and cycles
  build-up, a 3–5 s peak, a fade, and a 30–45 s relax.
- **Take-Two Interactive, US 11 071 916 B2 (filed 24 April 2019, granted 27 July 2021) and the
  continuation US 11 684 855 B2 (granted 27 June 2023), "System and method for virtual navigation
  in a gaming environment", Simon Parr and David Hynd** (<https://patents.google.com/patent/US11071916B2/en>).
  The claims: NPC routes over a coarse graph of the road nodes, so only nodes near a route's ends
  need be in memory, and in the continuation, route building on a server. The description adds
  ambient vehicles spawned to a target density per road link, counted along the network rather than
  radially, around the player's position and view direction.
- **NVIDIA, "Grand Theft Auto V PC Graphics & Performance Guide", 21 April 2015**
  (<https://www.nvidia.com/en-us/geforce/news/grand-theft-auto-v-pc-graphics-and-performance-guide/>).
  Population density changes only after the player travels far; pedestrians stay at short and medium
  range; distant vehicles are cheap stand-ins promoted or removed by density; variety is the number
  of models held in VRAM; "High Detail Streaming While Flying" lowers detail in the air (about 4 fps
  in the benchmark).
- **Sandy MacPherson (Kythera AI), "Cities at Scale: Simulating Street Life on a Budget", GDC 2022**
  (abstract, <https://gdcvault.com/play/1027576/AI-Summit-Cities-at-Scale>): traffic scheduled a
  short time ahead rather than stepped every frame, which reshapes spawning and LOD.
- Crowd tiers with numbers (Assassin's Creed Unity's 40 full AIs in 10 000) and Chenney's
  consistency and completeness tests are in `game-ai-ecosystems.md` §3.

*Bearing:* population budgets are counts per zone or per link, spawns go where nobody looks, the pool
is fixed. None of it needs learning; in a shared world it needs every player's view on the server.

---

## 4. Scheduling under a frame budget

**Time-slicing.** Unreal bounds asynchronous loading, actor initialisation and component
registration to milliseconds per frame, with extra time for high-priority loads and an optional
warning when a slice overruns (project settings, 5.8,
<https://dev.epicgames.com/documentation/unreal-engine/streaming-settings-of-the-unreal-engine-project-settings>).
Decima's and Ghost of Tsushima's per-frame defragmentation budgets (16 and 25 MiB) are in
`memory-streaming.md`. Forge's rules are of the same kind: `Low` jobs under 200 µs (D-005),
`upload_pages` per frame (D-025), one 16 MiB staging chunk (#220).

**The worst frame.** Epic's tech blog (4 February 2025; search record, the page refused the fetch)
describes shader-pipeline compilation at first draw as a stutter source and predicts pipeline states
when components load (PSO precaching, UE 5.2); Forge compiles its programs' entries ahead behind the
loading screen (#25, #200). Alen Ladavac (Croteam, "The Elusive Frame Timing", GDC 2018; search
record) showed games holding 60 fps that still stutter because they time rendering instead of
presentation. Vulkan now exposes presentation times and per-present target times through
`VK_EXT_present_timing` (Khronos blog, 4 December 2025,
<https://www.khronos.org/blog/vk-ext-present-timing-the-journey-to-state-of-the-art-frame-pacing-in-vulkan>;
NVIDIA developer drivers first, Mesa and Android to follow).

**Forge's spikes** (`docs/PROFILE.md`, `docs/demos/planet.md`). GPU frames are 0.7–1.5 ms on the
planet and about 1.3 ms on the island's tour at 1600 × 900: a tenfold margin on average. The losses
are spikes: 5–10 ms frames publishing edits of 210 and 314 tiles, an unexplained 22.5 ms frame with
a 7-tile edit, 6–8 ms frames without changes, one 49 ms frame on the island's tour. A governor that
lowers resolution would touch none of them; slicing the work that causes them, and naming each
spike's cause, would.

---

## 5. GTA VI and Rockstar

**What is public.**
- **The date.** Take-Two's results for the quarter to June 2026 (SEC 8-K, 7 August 2026,
  <https://www.sec.gov/Archives/edgar/data/0000946581/000162828026054580/ttwo1q27earningsrelease.htm>)
  list GTA VI on PS5 and Xbox Series X|S for 19 November 2026, after delays from 2025 and from
  26 May 2026. Nothing has shipped to analyse.
- **The trailers.** Trailer 2 (May 2025) was captured on a base PS5 (Rockstar, as reported). Digital
  Foundry, as reported by Pure Xbox on 8 May 2025, praised the ray-traced lighting and reflections,
  placed the resolution between 1080p and 1440p before reconstruction, and expected 30 fps. Nothing
  public describes its streaming, LOD or population systems.
- **Rockstar's own publications.** One engine talk: Fabian Bauer, "Creating the Atmospheric World of
  Red Dead Redemption 2", SIGGRAPH 2019 Advances (<https://advances.realtimerendering.com/s2019/index.htm>).
- **GTA V's knobs** (NVIDIA's guide, §3): distance scaling as the LOD setting; population density and
  variety as separate budgets; streaming detail lowered while flying; pauses when VRAM limits are
  ignored. Digital Foundry's 2013 reading of the Xbox 360 version (search record): it streamed from
  disc and hard drive at once for bandwidth.
- **Patents reached:** the two navigation patents of §3. Other Take-Two patents fan sites attach to
  GTA VI (interiors, locomotion) were not opened and are not cited.

**What is speculation.** A system that "optimises every scene" in GTA VI is documented nowhere. The
record supports less: RAGE has exposed distance scaling, density budgets and speed-dependent
streaming since GTA V; Take-Two's navigation patents describe density-driven spawning by view;
ray-traced lighting at 30 fps on a PS5 implies temporal accumulation and reconstruction. Any learned
component is unknown.
*Bearing:* the verifiable lesson is heuristic and old: separate budgets for distant detail,
population and variety, and less detail while the player moves fast.

---

## 6. Learned models on the client

**Upscaling, frame generation, ray reconstruction** are in `lighting-gi.md` §9. Since then:
- **DLSS 5** (NVIDIA GeForce News, 1 September 2026, <https://www.nvidia.com/en-us/geforce/news/dlss-5-3d-guided-neural-rendering/>):
  a final neural pass that adds lighting and material detail (subsurface scattering, light through
  hair, contact shadows) from the frame's colour, motion vectors, albedo and normals; RTX 50 only;
  deterministic for identical input frames; per-scene models, strength controls and masks. It shipped
  on 3 September 2026 in NBA 2K27 only, to disputes over changed art direction (press, search record).
  `lighting-gi.md` §9's "no DLSS 5 announced" is out of date.
- **PSSR's replacement**, the network co-developed with AMD under Project Amethyst (FSR 4's
  counterpart on PC), came to PS5 Pro in 2026 (Cerny to VGC, 2025,
  <https://www.videogameschronicle.com/news/mark-cerny-says-ps5-pro-will-get-its-major-replacement-for-the-current-pssr-upscaler-next-year>;
  March per press). Console only.
- **AMD's ML upscaler** still has no native Vulkan path found (search record); FSR 3.1 stays the open
  Vulkan one.
These are presentation only; their failures are ghosting and disocclusion errors. The NVIDIA RTX
SDKs licence (23 February 2024, as shipped with RTXNTC) names no royalty, limits DLSS to NVIDIA GPUs,
and asks applications using DLSS to show NVIDIA's marks, splash screens included: a point for the
owner's no-splash rule at a public release.

**Neural texture compression. NVIDIA RTXNTC SDK v0.10 beta; Karthik Vaidyanathan et al.,
"Random-Access Neural Compression of Material Textures", SIGGRAPH 2023; Laurent Belcour, Anis
Benyoub, "Hardware Accelerated Neural Block Texture Compression with Cooperative Vectors", HPG
2025.** [code] [paper]
<https://github.com/NVIDIA-RTX/RTXNTC> · <https://research.nvidia.com/labs/rtr/neural_texture_compression/> ·
<https://arxiv.org/abs/2506.06040>

A small network per material bundle decodes texels on demand: per sample in the shader (smallest
VRAM, fast only with cooperative vectors), at load into BCn (saves disk and PCIe, not VRAM; any
Shader Model 6 GPU), or per tile from sampler feedback (broken on AMD per the README). A 2k × 2k
bundle without mips: 32 MB raw, 12 MB BCn, 2.5 MB NTC on disk. Cooperative vectors bring 2–4 × over
the best non-tensor path; the DP4a fallback is for validation; Vulkan uses
`VK_NV_cooperative_vector`. Press puts sample mode at about 0.5–0.7 ms on an RTX 5070 at 1440p
(search record). The paper claims 16 × the texels at low bitrates. Belcour and Benyoub decode into a
block-compressed form the hardware filters: 28 MB per 4K nine-channel set at 0.55 ms on an Intel
B580. Licence: the same RTX SDK licence, credit required.
*Bearing:* decode-at-load is the cross-vendor use with a fixed cost: one more codec in Forge's cache
and packages (D-053, D-054).

**Caches, materials, visibility.** NRC, SHaRC and neural materials are in `lighting-gi.md` §1 (NRC
about 2.6 ms at 1080p in 2021, trained online, nothing on disk). Jakub Bokšanský and Daniel Meister
(AMD), "Neural Visibility Cache for Real-Time Light Sampling" (JCGT 2025, arXiv 2506.05930), learn
light visibility online to pick lights beside ReSTIR. These adapt every frame and ship no model.

**Deformers, animation, physics.** Unreal's ML Deformer (5.8,
<https://dev.epicgames.com/documentation/en-us/unreal-engine/ml-deformer-framework-in-unreal-engine>)
is trained in the editor from DCC simulations; its Neural Morph model evaluates a small network on
the CPU into morph-target weights (64–256 targets a character on slow GPUs); the GPU-only Vertex
Delta model is a reference. Learned motion matching is in `animation.md` §2 (a memory saving over a
database Forge does not have). Daniel Holden et al., "Subspace Neural Physics" (SCA 2019, Ubisoft La
Forge, <https://theorangeduck.com/page/subspace-neural-physics-fast-data-driven-interactive-simulation>),
run cloth and deformables 300–4000 × faster than the simulations they learned.
*Bearing:* all sit on top of D-012's server-owned parameters, as presentation.

**Learned LOD, impostors, occlusion.** Research exists (Towaki Takikawa et al., "Neural Geometric
Level of Detail", CVPR 2021; search record); no shipped game using learned LOD selection, impostors
or occlusion culling was found.

**The hardware path, October 2026.**
- **Vulkan:** `VK_NV_cooperative_vector` (per-invocation matrix-vector products for small MLPs,
  tolerant of divergence) is NVIDIA's and unratified; no KHR or EXT successor was found.
  `VK_KHR_cooperative_matrix` (2023) is cross-vendor; RADV merged it for RDNA 4 in February 2025
  (search record); the RX 9070 XT's Windows driver was not checked. Integer dot products
  (`VK_KHR_shader_integer_dot_product`) are core since Vulkan 1.3, accelerated or not as the device
  reports.
- **DirectX:** cooperative vectors came with Shader Model 6.9; Microsoft's post of 12 March 2026
  (<https://devblogs.microsoft.com/directx/evolving-directx-for-the-ml-era-on-windows/>) adds a
  Linear Algebra API with matrix-matrix operations, in preview from April 2026; Intel and NVIDIA
  state support, AMD is quoted without a commitment.
- **For Forge:** a learned pass is either optional (vendor SDK or NVIDIA extension) or plain compute
  with fp16 or int8 dot products, so the 9070 XT runs it.

| Technique | Cost per frame (published) | On disk / in memory | Cross-vendor in Vulkan |
|---|---|---|---|
| DLSS SR/RR/MFG, DLSS 5 | vendor tables not reached | driver DLLs | no |
| FSR 3.1 / FSR 4 | not reached | source / DLL | FSR 3.1 yes; FSR 4 no Vulkan |
| NRC (RTXGI) | ~2.6 ms at 1080p (2021) | trained online | NVIDIA path; SHaRC cross-vendor |
| NTC on sample / on load | ~0.5–0.7 ms at 1440p, RTX 5070 (press) / at load | 2.5 MB vs 12 MB BCn per 2k bundle | on load yes |
| Neural block textures | 0.55 ms at 1080p, Intel B580 | 28 MB per 4K set | yes |
| ML Deformer (Neural Morph) | CPU network + morph targets | network in RAM, targets in VRAM | yes (CPU) |

---

## 7. Learned models for prediction and control

**Camera and player prediction:** §1. Forge records inputs and replays them (`forge-sim`), so it can
log the paths and the misses a model would need before anyone trains one.

**Budget controllers.** Seyeon Kim et al., "zTT" (MobiSys 2021, best paper; search record), held a
rendering app's frame rate on a hot phone with 23.9 % less power by deep RL over CPU and GPU clocks.
It solves thermals on phones; on a desktop GPU the governors of §2 suffice.

**Directors and difficulty.**
- **Left 4 Dead** (§3) is the heuristic reference.
- **Su Xue, Meng Wu, John Kolen, Navid Aghdaie, Kazi Zaman (EA), "Dynamic Difficulty Adjustment for
  Maximized Engagement in Digital Games", WWW 2017 Companion**
  (<https://archives.iw3c2.org/www2017/proceedings/companion/p465.pdf>). Progression as a graph of
  (level, attempt) states with measured churn; dynamic programming picks the win rate per state that
  maximises rounds played. The lever was the board's random seed: measured win rates per seed ran
  from 0.15 to 0.75 on one level, and a server service returned one of the five best seeds for the
  target. Over three A/B phases rounds rose 4.4–7.9 % and play time 4.4–9.0 %, spending unchanged.
- **Zhengxing Chen et al. (EA), "EOMM", WWW 2017 (search record):** pairs players to minimise
  predicted churn rather than to equalise skill; gains in simulation; publicly criticised.
*Bearing:* EA's pattern keeps D-016: the model picks a seed on the server, the simulation stays a
deterministic function of it, the replay records the choice.

**Testing and balancing agents.** EA SEED put RL agents beside scripted bots to find exploits, stuck
spots and coverage gaps (Bergdahl et al., CoG 2020, search record) and used them on Battlefield 2042
and Dead Space (Gillberg et al., CoG 2023,
<https://ea.com/seed/news/cog23-challenges-deploying-rl-agents-game-testing>), the gap between
development builds and the final game being a main difficulty. King's CNN, trained on players'
moves, predicts new Candy Crush levels' difficulty better than MCTS at a fraction of the compute
(Gudmundsson et al., CIG 2018, search record).
*Bearing:* learning that never ships is the safest kind, and Forge's deterministic replays make every
bug a bot finds reproducible.

---

## 8. The server side

**Interest and priority.** Fiedler's accumulator, the Replication Graph (Fortnite: 100 players, about
50 000 replicated actors), D-010's 24 KB/s and EVE's time dilation are in `netcode.md` §1 and §3.
Epic's Iris (5.8, <https://dev.epicgames.com/documentation/en-us/unreal-engine/iris-prioritization-in-unreal-engine>)
spells the accumulator out: under 1.0 an object is skipped this tick; priorities accumulate until
sent, then reset; the default sphere prioritizer gives 1.0 inside, 0.2 outside, 0.1 beyond; there
are owner-boost, count-limit and field-of-view variants (a view cone, spheres and a line-of-sight
capsule, the highest winning). Mass computes a replication LOD per entity and client (§3).
*Bearing:* the server knows each client's camera from its inputs, so D-010's priority can take a
view-cone term: a dot product per entity and client.

**Server meshing and load.** Star Citizen's chairman's letter of 27 August 2026 (mirror,
<https://starcitizen.tools/Comm-Link:Letter_from_the_Chairman_-_2026-08-27>) calls Alpha 4.10's
instancing on demand the first use of dynamic server meshing: servers start when instance load
crosses a threshold, but territories are not yet subdivided, and one server per territory still
slows under crowds. SpatialOS's record is in `netcode.md` §3. Open autoscaling is heuristic: Agones
keeps a buffer of ready servers, switches policies on schedules and calls webhooks for custom logic
(<https://agones.dev/site/docs/reference/fleetautoscaler/>); Google's predictive autoscaling learns
daily and weekly cycles from 3 days to 3 weeks of CPU history, CPU only
(<https://docs.cloud.google.com/compute/docs/autoscaler/predictive-autoscaling>). Nae, Iosup et al.
(SuperComputing 2008) provisioned MMOG data centres from load predictions (abstract by search record).
*Bearing:* for one to sixteen players on one server none of this applies; a ready-server buffer comes
first when it does.

**ML anti-cheat and other server models.**
- **VACnet.** John McDonald (Valve), "Robocalypse Now: Using Deep Learning to Combat Cheating in
  Counter-Strike: Global Offensive", GDC 2018 (title by search record; figures from KitGuru and
  comicbook.com, 26 March 2018). It learns cheating from match behaviour, aimbots first, and submits
  cases to human reviewers: 80–95 % of its cases convict, against 15–30 % of players' reports. About
  1 700 CPUs carry the daily load; 3 456 were bought.
- **Toxicity.** Activision's October 2024 report (search record) credits machine-flagged voice
  moderation (Modulate's ToxMod) with 43 % less exposure to disruptive voice chat; people enforce.
- **Matchmaking:** EOMM (§7), measurable and contested.
*Bearing:* every server model that worked kept a person or an experiment between the model and the
penalty, ran off the tick, and fed on logs. Forge's replays and digests are that log.

---

## 9. Determinism and trust

D-016 requires the same bits from client and server in generation and simulation; D-056's 🟡
proposal adds that GPU code is authoritative only as integer noise or tightly constrained float,
checked by digests on two vendors. The systems above fall into three classes:

| Class | Examples | Rule |
|---|---|---|
| **Presentation (client, free to differ)** | page and tile prefetch, LOD thresholds, dynamic resolution, VRS, upscalers, DLSS 5, NTC, NRC, ML deformers, animation LOD, audio voices | may read timing, the camera and float networks; never feed the simulation; fixed modes in golden captures |
| **Simulation (both sides, deterministic)** | simulation LOD tiers, spawns, population counts, NPC decisions, physics LOD | pure functions of simulation state and seeds, never the client's camera or frame time; integer or `dmath`; digests at one and six workers |
| **Server-only decisions** | interest and priority, scaling, matchmaking, difficulty, anti-cheat, directors | may be learned or float; outputs enter the simulation as recorded inputs (a seed, a spawn command) |

How a trained model lives with D-016:
- **Presentation:** anything goes. PyTorch itself states that results are not reproducible across
  releases, platforms, or CPU and GPU (2.14 notes, <https://docs.pytorch.org/docs/2.14/notes/randomness.html>).
- **Offline:** a model tunes heuristics, writes tables, or tests; the table ships, not the model.
- **Server:** a model chooses among deterministic options, as EA's chose a seed; the choice is a
  logged input.
- **Simulation:** integer inference only. Benoit Jacob et al. (arXiv 1712.05877) quantise networks to
  integer-only arithmetic. Int8 products summed into int32 with wrapping addition give the same bits
  in any order, so a fixed-point MLP with integer requantisation matches across vendors; the
  saturating dot-product variants do not have that property and must be avoided. A float network on
  tensor cores does not match, because accumulation order and precision differ.

Out, by D-016: learned NPC policies or physics on the tick (as `game-ai-ecosystems.md` ruled), float
inference feeding anything replicated, generative passes such as DLSS 5 in golden captures, and any
budget that reads frame time and changes the simulation.

---

## What professional engines do

| Engine / game | What it predicts or ranks | Signal | Heuristic or learned | Client / server | Published gain |
|---|---|---|---|---|---|
| Unreal World Partition | cells to load | source position, priority, heading, velocity | heuristic | client | — |
| Unreal Cinematic Prestreaming | VT and Nanite pages of a sequence | requests recorded offline | recorded | client | — |
| Roblox | regions to send | foci radii; respawns, teleports | heuristic | server → client | — |
| Marvel's Spider-Man | 128 m tiles ahead | player position and travel | heuristic | client | ~1 tile/s at top speed |
| Ghost of Tsushima | mips, future cameras | screen coverage, cutscenes, camera cuts | heuristic | client | 1 GB texture budget |
| GTA V (RAGE) | detail, population, variety | distance scale, density, altitude | heuristic | client | ~4 fps (flight detail) |
| Take-Two patent | vehicles per road link | position, view direction, link density | heuristic | client / server | — |
| Left 4 Dead | enemies, pacing | intensity, visibility, travel distance | heuristic | server | hundreds of enemies, few entities |
| Unreal dynamic resolution | screen percentage | past GPU time vs budget | heuristic | client | — |
| Call of Duty: IW | AA quality, not resolution | GPU load | heuristic | client | — |
| Valve Lab renderer | resolution, MSAA | GPU timing | heuristic | client | 90 fps on a GTX 680 |
| Gears 5 / Tactics | shading rate per tile | last frame's luminance edges | heuristic | client | 8–20 % GPU at 4K |
| PS VR2 (Unity) | foveation | eye gaze | heuristic | client | up to 3.6 × |
| Unreal Animation Budget Allocator | tick rate, interpolation | significance, ms budget | heuristic | client | — |
| Unreal Mass | render, sim, replication LOD | distance, frustum, counts | heuristic | both | — |
| Unity Adaptive Performance | LOD bias, scale, frame rate | thermal, bottleneck | heuristic | client | — |
| Unreal Iris | replication priority | distance, view cone, owner, accumulation | heuristic | server | — |
| Star Citizen 4.10 | servers per instance | load threshold | heuristic | server | — |
| VACnet | cases to review | match behaviour | learned | server | 80–95 % vs 15–30 % convictions |
| EA difficulty service | board seed | progression, churn | optimised from data | server | +4–9 % rounds, play time |
| EA EOMM | match pairs | predicted churn | learned | server | simulated only |
| EA SEED, King | exploits, level difficulty | RL reward, imitation | learned | offline | used on BF2042, Dead Space |
| DLSS, FSR 4, PSSR | pixels | jittered frames, motion, depth | learned | client | `lighting-gi.md` §9 |
| NRC, NTC | radiance, texels | online MLP, per-material MLP | learned | client | 2.5 vs 12 MB per 2k bundle |
| Call of Duty voice moderation | toxic voice | audio | learned | server | 43 % less exposure |

The pattern: what runs every frame and decides what the player sees is a heuristic with a budget;
the learned systems that shipped produce pixels or advise a server-side decision that a person or an
experiment checks.

---

## Recommendation for Forge

**(a) Now: cheap heuristics for the planet and the streaming that exists.** Each is an issue under
D-056 step 1, D-018 or the profiling rules; none needs a new decision.
1. **A free-flight lead for the tile cut** (D-056 step 1's "prefetch along the velocity"). When the
   camera flies free, `focus` adds `position + velocity × latency`, velocity smoothed over about
   0.5 s, the same 0.25–4 s clamp. Next step: a scripted "free" path at three speeds, counting tiles
   that arrive after their split would show.
2. **The speed cap** (`planet-terrain.md` §4, GTA V's flight rule): the finest level asked for drops
   while `speed × latency` exceeds a cell's reach, and returns as the camera slows. Next step: the
   cut takes the speed; the golden shots, held still, must not change.
3. **Two classes of page request.** Demand and speculative queues on the I/O thread; speculative needs
   at half the demand scale, never evicting a page wanted this frame, at most a quarter of
   `upload_pages`. Sources: the start-view metric (#121) run on a worker for a predicted camera (the
   scripted pose ahead on tours, dead reckoning in free flight), and a guard band (the instance
   frustum test widened by angular speed × latency) in the need pass only, whose extra cull work is
   to be measured. Next step: an A/B on the tour at 300 m/s; the capture batch must stay 0 px (held
   views read nothing past their start view).
4. **Sliced edits.** An edit of hundreds of tiles goes in batches, ordered by the swap rule's error in
   pixels and by presence in the predicted frustum, with a per-frame cap on `scene/edit` bytes and on
   structure builds. Target: no frame over twice the p50 when arriving at Mont Blanc (5–10 ms today).
5. **A spike recorder.** A ring of per-frame counters (edits, page reads and uploads, staged bytes,
   BLAS and TLAS builds, shader compiles, worker queue depth); a frame over max(2 × p50, 4 ms) dumps
   the ring with a Tracy mark (headless capture, as for #220's staging stall). Goal: name the 22.5 ms
   and 49 ms frames. This is the "stay fluid" system Forge needs first.
6. **Stale cooks cancelled:** a tile job whose cell left the latest predicted cut is dropped before
   cooking; cooks run in order of error in pixels.

**(b) The engine later: budgets, significance, schedulers.**
1. **A significance service in `forge-sim`** (Phase 3), after Unreal's allocator and Mass: one score
   per entity from distance, screen size, last-frame visibility and gameplay boosts; millisecond
   budgets per category (animation, AI, physics proxies, audio voices, effects) with pressure
   thresholds, smoothing and state-change throttles; a client score from the camera for
   presentation and a server score from simulation state only. Decision: a D-057 🟡 that also adopts
   §9's class table.
2. **A frame-time governor, off by default:** Unreal's rules driving the TAA or DLSS input size,
   fixed in scripted runs, built only when a scene's p99 misses its target on the 5070 Ti (none does
   today). Part of D-057.
3. **Software VRS in the resolve** (Drobot), per 8×8 tile from last frame's luminance gradient and
   motion, when `shading/*` passes pass about 30 % of a 1440p frame.
4. **Present timing:** probe `VK_EXT_present_timing` and pace to it; meanwhile measure
   present-to-present jitter.
5. **Population** (`game-ai-ecosystems.md` §3's tiers): spawn in cells no player sees, behind by
   travel distance, from a fixed pool, density per path link. Take-Two's patents claim coarse-graph
   route finding (the density spawning is in their description only); a Forge route planner of that
   shape should be checked against the claims before a release.

**(c) Learned models: where they fit, the path, what stays out.**
1. **Vendor upscalers stay optional** behind the existing interface: DLSS through Streamline, FSR 3.1
   on Vulkan for AMD, FSR 4 when it has a Vulkan path; DLSS 5 only as an opt-in look, never in
   captures.
2. **NTC decode-at-load as a cache codec experiment** for the planet's colour maps and the material
   textures: disk and load time, not VRAM; measure against D-054's LZ4 and BCn before deciding.
3. **Neural caches** stay `lighting-gi.md`'s call (SHaRC cross-vendor, NRC on NVIDIA).
4. **Any in-house network** is plain compute with fp16 or int8 dot products; `VK_NV_cooperative_vector`
   and `VK_KHR_cooperative_matrix` are optional accelerations probed at start.
5. **Test bots before predictors:** scripted flyers and walkers over the planet tour and the island
   from `forge-sim` recordings, hunting late tiles, holes, spikes and stuck states; an RL explorer
   (EA SEED's shape) once the scripted ones run in CI; learned camera prediction only if logged
   misses show dead reckoning failing.
6. **Out, by D-016:** learned NPC policies and physics on the tick, float inference feeding the
   simulation, models deciding anything both sides compute. Decision: D-057's class table, amending
   D-016 with §9's integer rule.

**(d) The server.**
1. **Interest:** D-010's cells and accumulator as planned, plus Iris's view-cone term from each
   client's last camera; measure bytes per client against 24 KB/s with and without it.
2. **Server significance** reads simulation state only and sets simulation LOD tiers; a client's
   significance shapes only its presentation.
3. **Scaling:** one server now; a ready-server buffer when there are many; predictive scaling only
   with weeks of history.
4. **Server models** (cheats, toxicity, matchmaking, difficulty) wait for a game with players; they
   will run off the tick on recorded replays, their outputs recorded as inputs, with a person or an
   A/B test before any penalty. Now: keep replays and digests complete.

---

## What to measure

- **Lateness:** per second of flight, tiles and pages needed (over the pixel threshold) but not
  resident; time from first need to residency, p50/p99; prediction on and off, at 30, 300 and
  3 000 m/s.
- **Prediction quality:** speculative pages and tiles used within 2 s (precision); needed pages a
  speculation had already brought (recall); bytes and cooks wasted.
- **Frames:** p50/p99/max GPU and CPU per scene; frames over 2 × p50 per minute, each with its
  recorded cause; present-to-present jitter.
- **Edits:** ask-to-ready p50/p99; bytes and structure builds per frame; the longest publishing frame.
- **Swaps:** the existing ꟻLIP of each change's first and settled frames, so faster streaming does
  not pop more.
- **Budgets (later):** milliseconds per category against budget; entities per tier; state changes per
  second.
- **Learned passes (if any):** milliseconds on the 5070 Ti and on the AMD path, VRAM, disk bytes,
  ꟻLIP against the reference; digests on two vendors for anything near the simulation.
- **Server (later):** bytes per client per second; seconds since an entity in view was last sent;
  tick p99 per partition.

---

## Checked and left out

- **Valve's adaptive-quality thresholds:** the GDC 2016 slides exceeded the fetch limit; abstract only.
- **id Software's dynamic resolution:** no talk or paper found.
- **The PS5's six I/O priorities:** press quoting Cerny; the talk was not opened.
- **Flight Simulator 2024 prefetch along a flight plan:** no Asobo statement found.
- **Rift Apart's loading internals:** no technical talk; marketing and a rival's opinion only.
- **GTA VI's engine:** no Rockstar talk, paper or interview on streaming, LOD or population;
  fan-attributed patents beyond the two navigation patents not opened. A 2017 livestream observation
  that GTA V relocates a fixed pool of pedestrians (GTA BOOM, read) is not evidence of design.
- **DLSS execution-time tables:** in the SDK's programming guide, not reached.
- **Unreal's virtual-shadow-map dynamic LOD bias:** a forum cvar not found in Epic's documentation.
- **Epic's PSO-precaching post, Activision's toxicity report:** refused the fetch; search records.
- **Learned occlusion culling, LOD selection or impostors in a shipped game:** none found.
- **A cross-vendor Vulkan cooperative vector; the RX 9070 XT's Windows cooperative matrices:** not
  found; not verified.
- **Intel's velocity- and luminance-adaptive VRS article:** 404.
- **Fraud and churn models:** out of scope for a game without players.

---

## Verification notes

Checked on 2026-10-10 with WebFetch and WebSearch only. "Read" means WebFetch returned the page
and the facts above come from it; "search record" means only the search engine's extract or a summary
of it was seen. Two PDFs (Booth 2009, Xue et al. 2017) were saved by the fetch tool itself and read
as text with `pdftotext`; nothing else was downloaded.

- **Read:** Epic 5.8 pages (World Partition, `FWorldPartitionStreamingSource`,
  `UCinePrestreamingGraphNode`, Dynamic Resolution, Animation Budget Allocator,
  `FAnimUpdateRateParameters`, Mass Gameplay, Iris prioritization, streaming settings, ML Deformer);
  PlayStation Blog (6 September 2018); Sony's Rift Apart editorial (undated); Microsoft GDK
  `DSTORAGE_PRIORITY` (updated 6 November 2025); arXiv 1803.02329, 1911.11702v3 (revised April
  2021), 2506.05930 (August 2025), 2506.06040 (June 2025), 1712.05877 (2017); Microsoft Research's
  Outatime page; Roblox streaming and enum pages; GDC Vault 1023522 and 1027576 (abstracts); the Lab
  renderer README; Advances 2017, 2019, 2020 indexes; DirectX blog (12 January 2021; 12 March 2026);
  Unity blog (22 February 2023); Unity 6.6 manual; Booth's slides (2009); Google Patents US11071916B2
  and US11684855B2; NVIDIA's GTA V guide (21 April 2015); GTA BOOM (20 January 2017); Shacknews (21
  May 2026); Take-Two's 8-K (7 August 2026); Pure Xbox (8 May 2025); Khronos's
  `VK_NV_cooperative_vector` proposal and `VK_KHR_shader_integer_dot_product` page; Khronos blog (4
  December 2025); RTXNTC README and licence (23 February 2024); NVIDIA's NTC paper page; VGC on
  Cerny (2025); NVIDIA GeForce News on DLSS 5 (1 September 2026); Holden's page (24 July 2019); EA
  SEED's CoG 2023 page; Xue et al. (WWW 2017); KitGuru and comicbook.com (26 March 2018); the
  starcitizen.tools letter (27 August 2026); Agones reference; Google Cloud predictive autoscaling;
  TU Delft record of Nae et al. (metadata); PyTorch 2.14 notes; Walker's thesis record (2021).
- **Search record only:** Cerny's six priorities (2020); Burton on Rift Apart (2021); Flight Simulator
  2024 bandwidth; Epic's PSO-precaching post (4 February 2025); Ladavac (GDC 2018); the VACnet talk's
  title; Activision's October 2024 report; EOMM; King (CIG 2018); EA SEED (CoG 2020); zTT; NGLOD;
  RADV's RDNA 4 cooperative matrix; AMD's ML upscaler on Vulkan (2026); NTC press benchmarks; DLSS
  5's reception; PSSR's March 2026 release; Digital Foundry on GTA V's Xbox 360 streaming (2013);
  Rockstar's statement that Trailer 2 was captured on PS5; the neural predictor in Nae et al.
- **From earlier Forge research, not re-read:** Nanite, Ghost of Tsushima, Horizon Zero Dawn, the
  Spider-Man GDC abstract (`memory-streaming.md`); World Partition, the Significance Manager, Mass
  (`large-worlds.md`); Fiedler, the Replication Graph, Star Citizen 2021–2024, SpatialOS, EVE
  (`netcode.md`); AC Unity, Chenney, GT Sophy (`game-ai-ecosystems.md`); NRC, neural materials, DLSS
  4/4.5, FSR 4, XeSS (`lighting-gi.md`); learned motion matching (`animation.md`); Flight
  Simulator's GDC 2022 talk (`planet-terrain.md`).
- **Corrections for other files:** `lighting-gi.md` §9 (DLSS 5 shipped in September 2026);
  `gpu-geometry.md`'s "Rockstar publishes nothing" (one talk, SIGGRAPH 2019, on the atmosphere).
- **Numbers to re-check before a spec:** NTC's per-frame cost (press, one GPU); VACnet's CPU counts
  (2018); NRC's 2.6 ms (2021 hardware). Forge's own numbers are from `docs/demos/planet.md` and
  `docs/PROFILE.md` as of 2026-10-10.
