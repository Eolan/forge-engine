# Research — Lighting, global illumination, shadows, sky and upscaling

> Status: v0.1, 2026-09-24. Companion to [../RESEARCH.md](../RESEARCH.md) (generative core) and, in the
> sister repo, `world/docs/research/media-lighting.md` (media model). Question answered: what does
> Forge build next, on top of the Hillaire 2020 atmosphere, cascaded shadow maps, TAA and DLSS-via-
> Streamline it already has, to light huge procedural worlds from a no-RT fallback up to hardware path
> tracing on an RTX 5070 Ti, with an RTX 3080 as the second target?

An annotated bibliography for the lighting side of the renderer: real-time GI families and where each
stands in 2026, path tracing as shipped in games, what is actually public about Rockstar's RAGE, shadows,
direct lighting at scale, sky/clouds/fog/night, reflections, ambient occlusion, upscaling and frame
generation, and the colour pipeline. Every entry is tagged with the kind of source — **[paper]**
peer-reviewed or preprint (said which), **[book]**, **[talk]** conference presentation, **[web]**
engineering write-up, vendor page or documentation, **[code]** a maintained SDK or repository — and with
how it has aged: **foundational** (old, still correct, still the thing to read), **still-current** (the
standard reference for its problem today), **recent** (2022 or later, where the field is now). Every
citation was checked against at least one reachable page on 2026-09-23; where the canonical page
(ACM DL, Wiley, JCGT, EG diglib) refused automated access, an author page, GitHub, arXiv, a course index
or a vendor page was used and the DOI recorded. Things that could not be confirmed are in
[Checked and left out](#checked-and-left-out), not silently dropped. Press coverage (Digital Foundry,
trade sites) is labelled as press wherever it is the only source.

> **State of the art in five sentences.** In 2026 real-time GI at the top end is ReSTIR — direct
> light, then one-bounce GI, then full path reuse — feeding an AI denoiser, with a radiance cache
> (neural on tensor hardware, hash-grid elsewhere) supplying the tail bounces; it ships in open worlds on
> RTX 40/50-class GPUs at playable frame rates only because DLSS 4.5's transformer super-resolution,
> Ray Reconstruction and multi-frame generation carry the budget. Below that tier the field has settled
> on probe volumes updated by rays (DDGI) or Lumen's SDF-traced surface cache, with radiance cascades the
> one genuinely new noise-free idea — proven in 2D in a shipped game, 3D in a three-month-old preprint.
> Hardware ray tracing became a *minimum* requirement in id Software's and MachineGames' 2024–2025
> games, and press analysis reads GTA VI's PS5 lighting as ray-traced GI throughout, so a small studio
> can require RT and keep the no-RT path for tools and low-end hardware rather than as the product.
> Shadows are still cascaded or virtual shadow maps for the sun with rays for contact detail; skies are
> Hillaire's per-frame LUTs, clouds are Nubis-style ray marches, fog is a froxel volume, and screen-space
> occlusion (GTAO, visibility bitmasks) fills the first metre. Upscaling is vendor ML behind one input
> contract (jitter, motion vectors, depth, exposure, masks), and for a Vulkan engine the cross-vendor
> choice is still FSR 3.1 or XeSS 2 because FSR 4 has no Vulkan path.

**Contents**

1. [Real-time GI families](#1-real-time-gi-families)
2. [Path tracing in shipped games](#2-path-tracing-in-shipped-games)
3. [Rockstar RAGE: what is actually public](#3-rockstar-rage-what-is-actually-public)
4. [Shadows and the visibility buffer](#4-shadows-and-the-visibility-buffer)
5. [Direct lighting at scale, light units, display transforms](#5-direct-lighting-at-scale-light-units-display-transforms)
6. [Sky, atmosphere, clouds, fog, night](#6-sky-atmosphere-clouds-fog-night)
7. [Reflections](#7-reflections)
8. [Ambient occlusion and screen-space indirect](#8-ambient-occlusion-and-screen-space-indirect)
9. [Upscaling, anti-aliasing, frame generation](#9-upscaling-anti-aliasing-frame-generation)
10. [Colour pipeline, camera, validation](#10-colour-pipeline-camera-validation)
- [Recommendation for Forge](#recommendation-for-forge)
- [Checked and left out](#checked-and-left-out)
- [Verification notes](#verification-notes)

---

## 1. Real-time GI families

The families are: a surface-cache-plus-probe gather that runs with or without hardware rays (Lumen),
probe volumes updated by rays (DDGI), reservoir resampling (ReSTIR), radiance cascades, voxel/SDF cone
tracing, and neural caches and materials. They are not exclusive — shipped path tracers stack three of
them — and the recommendation at the end is a ladder built from several.

### Lumen and probe volumes

**Daniel Wright, Krzysztof Narkowicz, Patrick Kelly. "Lumen: Real-time Global Illumination in Unreal
Engine 5." *SIGGRAPH 2022, Advances in Real-Time Rendering in Games* course, 2022.** [talk]
[still-current]
<https://advances.realtimerendering.com/s2022/index.html> (predecessors on the same site: Wright,
"Radiance Caching for Real-time Global Illumination", Advances 2021,
<https://advances.realtimerendering.com/s2021/index.html>; Wright, "Dynamic Occlusion with Signed
Distance Fields", Advances 2015, <https://advances.realtimerendering.com/s2015/index.html>)

Two tracers under one gather. The software path sphere-traces per-mesh signed distance fields merged
into a clipmapped global SDF; hits are shaded from a *surface cache* — lighting stored on "cards"
(axis-aligned projections of each mesh) that is updated incrementally and already contains last frame's
indirect light, so multi-bounce falls out of feedback. The hardware path traces the BVH and shades hits
from the same cache. The final gather is *screen probes* (one octahedral probe per 16×16 pixels, placed
on depth, importance-sampled from the previous frame) backed by a world-space *radiance cache* of probes
for the far field, then temporal filtering. The 2021 talk is the radiance-cache mechanics; the 2015 talk
is where the distance-field infrastructure began. The sibling 2022 talk "Ray Tracing Open Worlds in
Unreal Engine 5" (Netzel, Costa) covers keeping a BVH alive in a streamed world.
*Bearing:* the reference architecture for GI that must run without hardware RT and get better with it:
same gather, two tracers, one cache. Forge's fallback tier should be Lumen-shaped, not a separate
cheaper algorithm.

**Zander Majercik, Jean-Philippe Guertin, Derek Nowrouzezahrai, Morgan McGuire. "Dynamic Diffuse
Global Illumination with Ray-Traced Irradiance Fields." *Journal of Computer Graphics Techniques* 8(2),
2019, 1–30.** [paper] [still-current]
<https://morgan3d.github.io/articles/2019-04-01-ddgi/> (JCGT page, fetch-blocked:
<http://jcgt.org/published/0008/02/01/>; SDK: NVIDIA RTXGI-DDGI 1.3,
<https://github.com/NVIDIAGameWorks/RTXGI-DDGI>)

A regular grid of probes, each storing an octahedral irradiance map and a second map of mean and
mean-squared distance to the nearest surface; every frame each probe shoots a few hundred rays, results
are blended in, and shading does a trilinear probe lookup weighted by a Chebyshev visibility test
against the stored distances — which is what stops light leaking through walls. NVIDIA's RTXGI 1.x SDK
is this algorithm with probe relocation, classification and cascaded volumes, for D3D12 and Vulkan;
RTXGI 2.x (see Müller, below) replaced DDGI with radiance caches, and the 1.x code was moved to its own
repository rather than deprecated in place.
*Bearing:* the cheapest credible dynamic GI for the RTX 3080 tier and the natural far-field store for
a planet: probes in world-space clipmaps, updated with ray queries when RT exists and with SDF marches
when it does not.

### ReSTIR

**Benedikt Bitterli, Chris Wyman, Matt Pharr, Peter Shirley, Aaron Lefohn, Wojciech Jarosz.
"Spatiotemporal reservoir resampling for real-time ray tracing with dynamic direct lighting." *ACM
Transactions on Graphics* (SIGGRAPH) 39(4), 2020.** [paper] [foundational]
<https://benedikt-bitterli.me/restir/> (DOI 10.1145/3386569.3392481; course: Wyman et al., "A Gentle
Introduction to ReSTIR: Path Reuse in Real-time", SIGGRAPH 2023 Courses,
<https://intro-to-restir.cwyman.org/>)

ReSTIR DI. Per pixel, draw a few dozen candidate light samples, keep one by resampled importance
sampling in a *reservoir*, merge that reservoir with last frame's and with neighbours' — each merge is
O(1) and needs no light data structure — and trace one shadow ray. Reported 6–60× (unbiased) and
35–65× (biased) error reduction over prior methods with millions of dynamic emissive triangles. The
2023 course is the readable derivation of RIS, reservoirs and spatiotemporal reuse, and includes the
Cyberpunk 2077 integration talk (Kozlowski, De Francesco).
*Bearing:* how Forge does many lights in the RT tiers: a procedural city with tens of thousands of
emitters needs no light culling on the RT path, only a candidate sampler and reservoirs.

**Yaobin Ouyang, Shiqiu Liu, Markus Kettunen, Matt Pharr, Jacopo Pantaleoni. "ReSTIR GI: Path
Resampling for Real-Time Path Tracing." *Computer Graphics Forum* (High Performance Graphics) 40(8),
2021.** [paper] [still-current]
<https://research.nvidia.com/publication/2021-06_restir-gi-path-resampling-real-time-path-tracing>
(DOI 10.1111/cgf.14378)

Extends reservoir reuse to indirect light: the sample is now a secondary path vertex (position, normal,
outgoing radiance) stored per pixel and reused across time and neighbours with a Jacobian for the
change of solid angle; one bounce traced per pixel per frame. Reported 9.3–166× MSE reduction over
1-spp path tracing before denoising. This is the "ReSTIR GI" mode Cyberpunk 2077 added in patch 2.1
and what RTXDI 2.0 shipped.
*Bearing:* the middle rung of the ladder — one bounce traced, several bounces' worth of variance
reduction, with a radiance cache supplying the tail.

**Daqi Lin, Markus Kettunen, Benedikt Bitterli, Jacopo Pantaleoni, Cem Yuksel, Chris Wyman.
"Generalized Resampled Importance Sampling: Foundations of ReSTIR." *ACM Transactions on Graphics*
(SIGGRAPH) 41(4), 2022.** [paper] [still-current]
<https://research.nvidia.com/publication/2022-07_generalized-resampled-importance-sampling-foundations-restir>
(DOI 10.1145/3528223.3530158; follow-up: Zhang, Lin, Kettunen, Yuksel, Wyman, "Area ReSTIR: Resampling
for Real-Time Defocus and Antialiasing", SIGGRAPH 2024, DOI 10.1145/3658210,
<https://graphics.cs.utah.edu/research/projects/area-restir/>)

The theory: RIS with correlated inputs of unknown PDFs, shift mappings between pixels' path spaces,
variance bounds and convergence conditions, and the resulting ReSTIR PT — full-path reuse with random
replay and reconnection shifts, shading one path per pixel while capturing caustic-like transport. Area
ReSTIR (2024) extends the reservoir to the 4D film-and-lens domain so antialiasing and depth of field
are resampled instead of post-processed; it removes the pinhole-per-pixel assumption that fights hair,
foliage and normal-map detail.
*Bearing:* the top rung, and the reason to structure Forge's path tracer around reservoirs and shift
mappings from day one rather than bolting them on.

**NVIDIA. RTX Dynamic Illumination (RTXDI) 3.1, RTX Path Tracing (RTXPT) 1.8, RTX Mega Geometry and
the RTX Kit. GitHub, 2025–2026.** [code] [recent]
<https://github.com/NVIDIA-RTX/RTXDI> · <https://github.com/NVIDIA-RTX/RTXPT> ·
<https://github.com/NVIDIA-RTX/RTX-Kit> · <https://github.com/NVIDIA-RTX/RTXMG>

RTXDI is ReSTIR as a library — DI (1.x), GI (2.0), PT (3.0) — on D3D12 and Vulkan through NVRHI, with
shaders compiled by DXC to SPIR-V. RTXPT is the reference integration: a path tracer with RTXDI, Shader
Execution Reordering, Opacity Micro-Maps, NRD ReBLUR/ReLAX, stochastic texture filtering and Streamline
DLSS SR/RR/FG/MFG; D3D12 first, Vulkan with manual setup, under NVIDIA's own licence (v1.8.1). RTX Kit
(2025) is the umbrella: DLSS, Neural Shading, Neural Texture Compression, Texture Filtering and
Streaming, Mega Geometry, Character Rendering, RTXGI, RTXDI, RTXPT, Streamline, NRD, OMM, RTXMU, STBN;
it asks for Vulkan SDK ≥ 1.3.296. RTXMG shows cluster acceleration structures
(`VK_NV_cluster_acceleration_structure`) with streamed cluster LOD for geometry larger than VRAM and
per-frame rebuilt tessellated subdivision surfaces — 1.6 billion unique triangles in the sample.
*Bearing:* adopt the shader-side of RTXDI (reservoir and resampling headers) and read RTXPT as the
integration template; do not adopt NVRHI. RTXMG is the answer to "how does a procedural planet get a
BVH": cluster AS plus streaming, not a monolithic rebuild.

### Radiance cascades

**Alexander Sannikov. "Radiance Cascades: A Novel Approach to Calculating Global Illumination."
Self-published (JCGT template, never submitted), 2023.** [paper] [recent]
<https://github.com/Raikiri/RadianceCascadesPaper> (context: 80.lv, November 2023,
<https://80.lv/articles/radiance-cascades-new-approach-to-calculating-global-illumination>)

A probe hierarchy built on one observation: angular resolution must grow with distance while spatial
resolution can shrink. Cascade *i* stores probes at 2^i spacing with 4^i directions, each covering a ray
*interval* [2^i, 2^(i+1)); radiance is gathered by merging intervals from coarse to fine, so cost is
independent of light count and scene complexity, and a penumbra is resolved by the cascade whose
interval matches its distance. Sannikov is a senior programmer at Grinding Gear Games (Path of Exile),
where the 2D/2.5D form lights the game. The practical distinction from ReSTIR-style methods is that the
result is nearly noise-free without temporal accumulation.
*Bearing:* the best candidate for Forge's no-RT and far-field GI: deterministic, noise-free,
GPU-friendly and cascaded like the terrain clipmaps. Its 3D memory cost is the thing to test (next).

**Rouli Freeman, Alexander Sannikov. "Split Radiance Cascades: Real-Time Global Illumination via
Sparse Radiance Probes." arXiv 2607.20384, July 2026.** [paper] [recent] (preprint)
<https://arxiv.org/abs/2607.20384> (2D precursor: Freeman, Sannikov, Margel, "Holographic Radiance
Cascades for 2D Global Illumination", arXiv 2505.02041, May 2025, <https://arxiv.org/abs/2505.02041>)

The 3D problem with radiance cascades is storage — dense 3D grids with 4^i directions do not fit — so
this takes the cascades to full 3D with sparse hash maps and *ray splitting*: rays are traced from
visible surfaces and their contributions routed to the cascade level matching the hit distance. The
authors report high-quality indirect illumination both single-frame and temporally accumulated. The
2025 paper is the 2D version (1.85 ms at 512², 7.67 ms at 1024² on a consumer GPU) and the clearest
exposition of interval merging.
*Bearing:* this paper decides whether radiance cascades can be Forge's world-space GI or only a 2D
curiosity; it is three months old, so prototype rather than trust.

### Voxel and SDF cone tracing

**Cyril Crassin, Fabrice Neyret, Miguel Sainz, Simon Green, Elmar Eisemann. "Interactive Indirect
Illumination Using Voxel Cone Tracing." *Computer Graphics Forum* (Pacific Graphics) 30(7), 2011.**
[paper] [foundational]
<https://research.nvidia.com/publication/2011-09_interactive-indirect-illumination-using-voxel-cone-tracing>
(DOI 10.1111/j.1467-8659.2011.02063.x; descendant: Linietsky, "Godot 4.0 gets SDF based real-time
global illumination", June 2020,
<https://godotengine.org/article/godot-40-gets-sdf-based-real-time-global-illumination/>)

Voxelise the scene into a sparse octree with pre-filtered radiance and occupancy, inject direct light at
the leaves, mip it up, and trace cones by stepping through the mip levels — diffuse GI with a few wide
cones, glossy with a narrow one. It became NVIDIA VXGI and a generation of voxel-GI plugins, and its
costs are the known ones: memory for the volume, re-voxelisation for dynamic objects, leaking at thin
walls. Godot 4's SDFGI is the pragmatic descendant — cascaded SDF plus occlusion volumes plus probes,
each cascade doubling the voxel size, with walls required to be thicker than a voxel of their cascade.
*Bearing:* read for the failure modes, not to implement. Where Forge needs a coarse world-space store
for the far field, SDF-plus-probes (Lumen, DDGI, cascades) beats cone tracing on memory and leaks.

### Neural caches and neural shading

**Thomas Müller, Fabrice Rousselle, Jan Novák, Alexander Keller. "Real-time Neural Radiance Caching
for Path Tracing." *ACM Transactions on Graphics* (SIGGRAPH) 40(4), 2021.** [paper] [still-current]
<https://research.nvidia.com/publication/2021-06_real-time-neural-radiance-caching-path-tracing>
(DOI 10.1145/3450626.3459812; SDK: RTXGI 2.x with NRC and SHaRC, <https://github.com/NVIDIA-RTX/RTXGI>)

A small MLP maps (position, direction, normal, roughness, albedo…) to outgoing radiance and is trained
*online* every frame from the renderer's own longer training paths — "generalisation via adaptation" —
so short paths terminate into the cache after one or two bounces: about 2.6 ms at 1080p in 2021, no
pre-training, everything dynamic. RTXGI 2.x ships it (NVIDIA Tensor Cores from Turing, driver ≥ 555.85,
D3D12 and Vulkan) beside SHaRC, a world-space hash-grid radiance cache that runs on any DXR/Vulkan-RT
GPU and is the vendor-neutral stand-in. Both are meant to sit behind a ReSTIR path tracer and supply the
tail bounces.
*Bearing:* NRC is what "path tracing at 60 fps with multi-bounce" means in 2026; adopt SHaRC for the
cross-vendor path and NRC where cooperative vectors exist (both dev GPUs qualify).

**Tizian Zeltner, Fabrice Rousselle, Andrea Weidlich, Petrik Clarberg, Jan Novák, Benedikt Bitterli,
Alex Evans, Tomáš Davidovič, Simon Kallweit, Aaron Lefohn. "Real-Time Neural Appearance Models." *ACM
Transactions on Graphics*, 2024 (presented at SIGGRAPH 2024).** [paper] [recent]
<https://research.nvidia.com/labs/rtr/neural_appearance_models/> (DOI 10.1145/3659577; the Vulkan
extension: <https://docs.vulkan.org/features/latest/features/proposals/VK_NV_cooperative_vector.html>;
SDK: RTX Neural Shading, <https://github.com/NVIDIA-RTX/RTXNS>)

Materials as learned hierarchical latent textures decoded by small MLPs that output BRDF values *and*
importance-sampled directions, with graphics priors (learned shading frames, microfacet sampling) so the
network stays tiny; more than an order of magnitude faster than the layered materials it replaces, with
LOD built in. The enabling hardware is *cooperative vectors*: `VK_NV_cooperative_vector` lets each
shader invocation run its own matrix–vector multiplies on tensor hardware in the ordinary SIMT model,
unlike subgroup-wide cooperative matrices. RTXNS is the Slang-based SDK (training with Slang autodiff,
inference in shaders; RTX 20-series and newer on Vulkan). What is shippable now: NRC and neural texture
compression. Neural materials are shippable in principle and used by no shipped game I could verify.
*Bearing:* Forge already compiles Slang and has cooperative vectors on both GPUs; the first neural use
should be NRC, the second texture compression, and neural materials only once the material library is
stable.

---

## 2. Path tracing in shipped games

Five data points from 2023–2025, then the denoiser that makes them possible. The pattern is the same
every time: ReSTIR direct light, one-bounce ReSTIR GI or a cache for indirect, a denoiser (NRD at first,
DLSS Ray Reconstruction after mid-2023), and an upscaler plus frame generation to pay for it.

**CD PROJEKT RED / NVIDIA. "Cyberpunk 2077: Technology Preview of New Ray Tracing: Overdrive Mode Out
Now." GeForce News, 11 April 2023.** [web] [recent]
<https://www.nvidia.com/en-us/geforce/news/cyberpunk-2077-ray-tracing-overdrive-update-launches-april-11/>
(integration talk: Kozlowski, De Francesco, "ReSTIR Integration in Cyberpunk 2077", SIGGRAPH 2023
course, <https://intro-to-restir.cwyman.org/>; GDC 2023 announcement, <https://blogs.nvidia.com/?p=63236>)

The first AAA open world to replace its whole lighting pipeline with a path tracer: RTXDI (ReSTIR DI)
for direct light from every neon sign and headlight, NRD for denoising, Shader Execution Reordering for
incoherent hits, DLSS 3 for the frame budget; ReSTIR GI arrived in patch 2.1 and Ray Reconstruction
replaced NRD later in 2023. Recommended hardware at launch was an RTX 40-series. The 2023 course talk is
the honest engineering account — light sampling structure, bias controls, what broke. A GDC 2024 talk
("RT: Overdrive in Cyberpunk 2077 Ultimate Edition — Pushing Path Tracing One Step Further") is listed
on NVIDIA On-Demand but its page did not render for me.
*Bearing:* the proof that ReSTIR DI/GI plus a denoiser is a shipping product for a dense night-time
city — Forge's hardest lighting case — and the template for path tracing as an optional top tier.

**Remedy Entertainment / NVIDIA. "Alan Wake 2 Available October 27 With Full Ray Tracing & DLSS 3.5."
GeForce News, October 2023.** [web] [recent]
<https://www.nvidia.com/en-us/geforce/news/gfecnt/202310/alan-wake-2-dlss-3-5-full-ray-tracing-out-this-week/>
(talk: Sjöholm, Kandar, "Alan Wake 2: A Deep Dive into Path Tracing Technology", GDC 2024, listing at
<https://www.alanwake.info/2024/03/18th-march-2024-final-gdc-2024.html>)

Northlight's "full ray tracing": up to three bounces of direct and indirect light, full-resolution
opaque and transparent reflections, one unified system replacing SSR, SSAO and probe GI; shipped with
Ray Reconstruction as the denoiser and with raster presets below it for older RTX cards. Remedy rebuilt
Northlight around GPU-driven rendering and mesh shaders for this game. The GDC 2024 talk covers
acceleration-structure management with heavy dynamic content, material access from hit shaders, light
sampling, upscaling and denoising.
*Bearing:* the clearest public example of a mesh-shader, GPU-driven engine adding path tracing as a top
tier while keeping a raster ladder below it; Forge's plan is the same shape.

**MachineGames and id Software. *Indiana Jones and the Great Circle* (2024) and *DOOM: The Dark Ages*
(2025): hardware ray tracing as the minimum requirement.** [web] [recent]
<https://help.bethesda.net/app/answers/detail/a_id/66629/> ·
<https://store.steampowered.com/api/appdetails?appids=3017860> ·
<https://developer.nvidia.com/blog/how-id-software-used-neural-rendering-and-path-tracing-in-doom-the-dark-ages>

Both games list hardware ray tracing as required at every tier — RTX 2060 SUPER / RX 6600 / Arc A580
minimum for Indiana Jones; "hardware Raytracing-capable GPU with 8 GB" for Doom — because id Tech's GI
is ray-traced with no baked fallback. id's Billy Khan told Digital Foundry (May 2025, press) the game
"isn't possible" without RT, chiefly for iteration: no bakes. Both later added full path tracing;
Doom's took about six months and uses RTXGI, Opacity Micro-Maps, SER and DLSS 4 Ray
Reconstruction/Multi Frame Generation (NVIDIA blog, September 2025). The lesson in the requirements: a
2019 GPU is now the floor, and at that floor RT GI runs at 60 fps on consoles.
*Bearing:* Forge can make hardware RT the minimum with a clear conscience — the RTX 3080 is two
generations above the floor these games set — and keep the no-RT path for tooling, servers and a future
handheld, not as the primary product.

**Game Science / NVIDIA. "Black Myth: Wukong Out Now With Full Ray Tracing & DLSS 3." GeForce News,
20 August 2024.** [web] [recent]
<https://www.nvidia.com/en-us/geforce/news/black-myth-wukong-full-ray-tracing-dlss-3/> (announcement
with Ray Reconstruction, March 2024:
<https://www.nvidia.com/en-us/geforce/news/gdc-2024-dlss-rtx-full-ray-tracing-game-announcements/>)

A UE5 game whose "Full Ray Tracing" preset goes past Lumen's hardware path: two-bounce indirect light,
full-resolution reflections including a two-level trace for particles and transparents, ray-traced water
caustics, and contact-hardening shadows everywhere; about 100 fps at 4K max settings on an RTX 4090 with
frame generation.
*Bearing:* evidence that a mid-sized studio ships path-traced lighting on a licensed engine plus vendor
SDKs; the caustics and particle reflections are the extras that separate a path tracer from RT GI and
belong on Forge's PT-tier feature list.

**NVIDIA. "NVIDIA Real-Time Denoisers (NRD) 4.18." GitHub, 2026.** [code] [still-current]
<https://github.com/NVIDIA-RTX/NRD>

API-agnostic (D3D11/12, Vulkan) spatio-temporal denoisers guided by normals, roughness, depth and
motion vectors: REBLUR (recurrent blur, the default), RELAX (à-trous, tuned for RTXDI signals), SIGMA
(per-light shadows including sun/moon and translucency), with SH variants for diffuse/specular and
occlusion-only modes. It is the open, portable denoiser; Ray Reconstruction is better and faster on
NVIDIA but closed and vendor-locked; AMD's counterpart is FSR Ray Regeneration (§9).
*Bearing:* the denoiser for Forge's cross-vendor RT tiers and the yardstick for RR; design the G-buffer
(normal/roughness/depth/mvec, split diffuse and specular radiance, hit distance) to NRD's inputs from
the start.

---

## 3. Rockstar RAGE: what is actually public

Rockstar publishes almost nothing. There is one substantial primary talk, two third-party frame
captures, and press analysis of a trailer. That is the whole public record, and this section says so.

**Fabian Bauer. "Creating the Atmospheric World of Red Dead Redemption 2: A Complete and Integrated
Solution." *SIGGRAPH 2019, Advances in Real-Time Rendering in Games* course, 2019.** [talk]
[still-current]
<https://advances.realtimerendering.com/s2019/index.htm>

The one substantial primary source. It covers RDR2's sky, cloud and fog rendering, its volumetric
effects, and the ambient lighting model derived from them — indirect light from the sky as a function
of the same atmosphere the sky is drawn with — built to obey light transport yet stay directable for
time-of-day, weather and mood. It is not a GI talk: RDR2's world lighting is sky ambient plus baked
data. No Rockstar talk on GTA V, GTA VI or any RAGE GI system exists that I could find; "Rendering the
World of RDR2" is not a real title (see Checked and left out).
*Bearing:* what RAGE's famous atmosphere actually is — an integrated sky/cloud/fog/ambient pipeline.
Forge has the sky; the missing pieces are clouds, fog and the coupling of ambient light to both.

**imgeself. "Graphics Study: Red Dead Redemption 2." Blog, 19 June 2020.** [web] [still-current]
<https://imgeself.github.io/posts/2020-06-19-graphics-study-rdr2/> (predecessor: Adrian Courrèges,
"GTA V — Graphics Study", 2 November 2015,
<https://www.adriancourreges.com/blog/2015/11/02/gta-v-graphics-study/>)

Frame captures, so third-party but concrete. RDR2 (2018): a deferred renderer with six G-buffer
targets including baked AO and motion vectors; four 1024² sun cascades plus an atlas for spot and point
shadows; per-frame environment cubemaps with split-sum IBL for specular; baked large-scale AO and
"top-down world lightmaps" for static light; SSAO for detail; TAA; clouds and volumetrics per Bauer.
GTA V (2015): four sun cascades in a 1024×4096 atlas, dual-paraboloid environment reflections, half-res
SSAO, light shafts ray-marched against the shadow map, Uncharted-2 filmic tone mapping with adaptive
exposure, FXAA. Neither game has dynamic GI; both lean on atmosphere and baked or probe data.
*Bearing:* RAGE up to RDR2 is a conventional deferred pipeline executed extremely well. The bar Forge
must clear is art-directed atmosphere and exposure discipline, not an exotic algorithm.

**Rockstar Games. "Grand Theft Auto VI." Official site, 2026; and Digital Foundry, "Grand Theft Auto 6
Trailer 2 Tech Breakdown", May 2025 (press).** [web] [recent]
<https://www.rockstargames.com/VI> (DF's video relayed by press:
<https://libertycity.net/news/gta-6/8997-digital-foundry-analyzes-gta-6s-second.html>)

Primary source: Rockstar's site gives 19 November 2026 as the release date and says nothing about
rendering. Everything else is press. Digital Foundry's frame-by-frame reading of trailer 2 (widely
reported as captured on a PlayStation 5) concludes: ray-traced GI throughout with no rasterised
fallback, ray-traced reflections on small water bodies and screen-space reflections on large water,
glass and plastics, filtered non-RT shadows in the RDR2 style, strand-based hair, 30 fps, with press
summaries disagreeing on internal resolution (1152p–1440p). These are inferences from video; no
Rockstar engineer has described RAGE 9's GI.
*Bearing:* the credible reading — RT GI everywhere, hybrid reflections, raster shadows, on a 2020
console — is exactly the tier Forge's RTX 3080 target should hit. Treat it as a design target and cite
it as press.

---

## 4. Shadows and the visibility buffer

**Microsoft. "Cascaded Shadow Maps." DirectX technical article, rev. 2018.** [web] [still-current]
<https://learn.microsoft.com/en-us/windows/win32/dxtecharts/cascaded-shadow-maps> (soft shadows:
Fernando, "Percentage-Closer Soft Shadows", SIGGRAPH 2005 Sketches, PDF
<https://developer.download.nvidia.com/shaderlibrary/docs/shadow_PCSS.pdf>)

Still the best single write-up of what Forge already runs: frustum partitioning (fit-to-scene vs
fit-to-cascade), texel-snapped light matrices against shimmer, interval- vs map-based cascade selection,
blend bands between cascades, PCF kernels with derivative-based depth bias, and variance shadow maps as
the filterable alternative with their light-bleeding cost; it credits Engel's ShaderX5 (2006) chapter as
the origin. PCSS adds the blocker search, penumbra estimate and variable-kernel PCF that give
contact-hardening from a sun disc of known angular size.
*Bearing:* CSM stays as the sun-shadow path of the raster tier; add PCSS driven by the star's angular
diameter (Forge's stars are disc lights), and treat the per-frame cascade budget as what VSM or RT
shadows will replace.

**Epic Games. "Virtual Shadow Maps in Unreal Engine." UE5 documentation.** [web] [recent]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-shadow-maps-in-unreal-engine>

One 16k×16k virtual shadow map per light in 128×128 pages allocated only where the depth buffer needs
them; directional lights use a clipmap of such maps around the camera, local lights mip chains or cubes;
pages are cached across frames and invalidated only by moving casters or lights; rendering into them
relies on Nanite's GPU-driven multi-view rasterisation, and non-Nanite geometry is markedly more
expensive. Limits: bounds-dependent invalidation, projective aliasing at grazing light angles, page-pool
overflow with many casting lights.
*Bearing:* VSM is the shadow method that matches a visibility-buffer, cluster-culled renderer; Forge
should plan for it once its GPU-driven geometry path exists and use CSM until then.

**Eric Haines, Tomas Akenine-Möller (eds.). *Ray Tracing Gems*. Apress, 2019 (free PDF, CC
BY-NC-ND).** [book] [still-current]
<http://www.realtimerendering.com/raytracinggems/> (chapter list:
<https://www.realtimerendering.com/raytracinggems/rtg/index.html>)

The chapters that matter here: ch. 13, Boksansky, Wimmer, Bittner, "Ray Traced Shadows: Maintaining
Real-Time Frame Rates" — adaptive per-pixel sample counts driven by penumbra variance, temporal
filtering, and shadow maps for the far field with rays near the camera; ch. 25, Barré-Brisebois et al.,
"Hybrid Rendering for Real-Time Ray Tracing" (SEED's PICA PICA: raster G-buffer, RT reflections, shadows
and AO, a denoiser per signal); ch. 19 on UE4's RT denoising; ch. 26, "Deferred Hybrid Path Tracing".
Ray Tracing Gems II (2021) continues the series.
*Bearing:* the hybrid recipe for Forge's middle tier is ch. 25; the sun-shadow recipe (rays near,
cascades far, adaptive sample count) in ch. 13 is what to build the day RT shadows are switched on.

**Christopher A. Burns, Warren A. Hunt. "The Visibility Buffer: A Cache-Friendly Approach to Deferred
Shading." *Journal of Computer Graphics Techniques* 2(2), 2013, 55–69.** [paper] [foundational]
<https://jcgt.org/published/0002/02/04/> (PDF reachable, HTML page not; practitioner account: Engel,
"The Triangle Visibility Buffer", 2018, rev. 2021,
<https://diaryofagraphicsprogrammer.blogspot.com/2018/03/triangle-visibility-buffer.html>)

Rasterise only a triangle ID and draw/instance ID plus depth; at shading time fetch the three vertices,
recompute barycentrics and derivatives analytically, and shade — no fat G-buffer, bandwidth is one
32-bit target, and shading is a compute pass per material. Engel's series adds triangle filtering,
indirect draws with compaction and tiled ("Forward++") lighting on top.
*Bearing:* the geometry/shading split Forge wants for a GPU-driven mesh-shader renderer — visibility
buffer, material classification into tiles, then shading — and what lets VSM and RT hit-shading coexist
with raster.

**Brian Karis, Rune Stubbe, Graham Wihlidal. "A Deep Dive into Nanite Virtualized Geometry." *SIGGRAPH
2021, Advances in Real-Time Rendering in Games* course, 2021.** [talk] [still-current]
<https://advances.realtimerendering.com/s2021/index.html>

Cluster DAG LOD, GPU-driven two-pass occlusion culling, a software rasteriser for micro-triangles using
64-bit atomics, a visibility buffer with material classification and per-material shading, and the
multi-view rasterisation VSM relies on; frank about what does not fit (deformation, foliage,
transparency) and why.
*Bearing:* Forge has mesh shaders, 64-bit atomics and cluster BVHs on both machines; this talk is the
specification for how a visibility buffer, VSM and cluster-based RT (RTXMG) share one cluster
representation.

---

## 5. Direct lighting at scale, light units, display transforms

**Ola Olsson, Markus Billeter, Ulf Assarsson. "Clustered Deferred and Forward Shading." *High
Performance Graphics*, 2012.** [paper] [foundational]
<https://www.cse.chalmers.se/~uffe/clustered_shading_preprint.pdf> (DOI 10.2312/EGGH/HPG12/087-096;
production version: Persson, "Practical Clustered Shading", SIGGRAPH 2013,
<https://www.humus.name/Articles/PracticalClusteredShading.pdf>, index
<https://www.humus.name/index.php?page=Articles>)

Bin lights into 3D clusters of the view frustum (screen tiles × exponential depth slices, optionally ×
normal cone) and shade with the cluster's light list; unlike 2D tiles it does not break at depth
discontinuities, and it works for forward as well as deferred, which is what makes it the default for
transparents and for engines with many small lights. Persson's talk is the Avalanche production
version: grid layout, light-shape culling, and the observation that clustered forward removes most
reasons to stay deferred.
*Bearing:* Forge's raster tiers use clustered lighting for everything (opaque through the visibility
buffer, transparents forward); the RT tiers replace the cluster walk with ReSTIR DI but keep the grid as
the candidate-generation structure.

**Sébastien Lagarde, Charles de Rousiers. "Moving Frostbite to Physically Based Rendering." *SIGGRAPH
2014* course notes, v3.** [talk] [foundational]
<https://seblagarde.wordpress.com/2015/07/14/siggraph-2014-moving-frostbite-to-physically-based-rendering/>

The document that fixed the units: sun in lux, punctual lights in lumens or candela, emissives in nits,
sky in luminance; a physical camera (aperture, shutter, ISO → EV100) and pre-exposure so 100,000 lux and
a candle coexist in fp16; area lights (sphere, disc, rectangle, tube); specular occlusion from AO; and
the calibration workflow. Every later engine's "physical light units" page cites it.
*Bearing:* Forge already uses these units for the atmosphere; make them law for every light, emissive
and the camera, and keep the pre-exposure convention because DLSS and NRD both take an exposure input.

**Academy of Motion Picture Arts and Sciences / Academy Software Foundation. "ACES 2.0 core transforms
(aces-core)." GitHub, 2025.** [code] [recent]
<https://github.com/ampas/aces-core> (alternatives: Sobotka, "AgX", <https://github.com/sobotka/AgX>,
adopted as Blender 4.0's default view,
<https://developer.blender.org/docs/release_notes/4.0/color_management/>; Khronos Group, "PBR Neutral",
<https://github.com/KhronosGroup/ToneMapping>)

ACES 2.0 replaced the 1.x RRT/ODT with a new output transform — tonescale, chroma compression and gamut
compression as separable stages — under Apache 2.0. AgX is the hue-safe alternative games have gravitated
to: bright saturated colours desaturate toward white as film does instead of skewing hue (Blender's
stated reason for switching from Filmic). Khronos PBR Neutral is the opposite design point — keep base
colours faithful in the mid-range (linear, then compressed, desaturating only in highlights) for
product visualisation. All three are drop-in on the same scene-linear input.
*Bearing:* ship the display transform as data with AgX as the default look, ACES 2.0 for a film-style
look, PBR Neutral for the material-validation view, and never bake a transform into lighting.

**Jasmin Patry. "Real-Time Samurai Cinema: Lighting, Atmosphere, and Tonemapping in Ghost of
Tsushima." *SIGGRAPH 2021, Advances in Real-Time Rendering in Games* course, 2021.** [talk]
[still-current]
<https://advances.realtimerendering.com/s2021/index.html>

An open-world, day/night, weather-driven game's lighting stack in one talk: sky and atmosphere, clouds
and fog, sun and indirect light over a large outdoor world, exposure, and a tone-mapping design built to
keep saturated colours from hue-shifting under a filmic curve — the closest published analogue to
Forge's target (large outdoors, strong art direction, console budget).
*Bearing:* read beside Bauer 2019 as the two "how a big open world is lit" talks; both put atmosphere
and exposure before GI.

---

## 6. Sky, atmosphere, clouds, fog, night

**Sébastien Hillaire. "A Scalable and Production Ready Sky and Atmosphere Rendering Technique."
*Computer Graphics Forum* (EGSR) 39(4), 2020, 13–22.** [paper] [still-current]
<https://sebh.github.io/publications/egsr2020.pdf> (DOI 10.1111/cgf.14050; index
<https://sebh.github.io/publications/>; engine version: Hillaire, "Physically Based and Scalable
Atmospheres in Unreal Engine", SIGGRAPH 2020 Physically Based Shading course,
<https://blog.selfshadow.com/publications/s2020-shading-course/>; predecessor: Bruneton, Neyret,
"Precomputed Atmospheric Scattering", EGSR 2008, DOI 10.1111/j.1467-8659.2008.01245.x, 2017
reimplementation <https://ebruneton.github.io/precomputed_atmospheric_scattering/>)

Four small LUTs recomputed every frame — transmittance, a multiple-scattering LUT (an isotropic-bounce
approximation replacing Bruneton's 4D tables), a sky-view LUT, and a froxel aerial-perspective volume —
give a physically based sky with ozone and multiple scattering that runs from mobile to planet scale,
correct from the ground to orbit, with parameters editable live. Bruneton 2008 is the exact-but-
precomputed predecessor; his 2017 code adds ozone, spectral rendering and non-Earth parameters and is
the reference to validate against.
*Bearing:* already in Forge. What remains is the coupling — cloud shadows and cloud-scattered light
into the aerial perspective, the sky-view LUT as the ambient source for GI probes — and validation of
extreme atmospheres against Bruneton's code.

**Andrew Schneider. "Nubis³: Methods (and Madness) to Model and Render Immersive Real-Time Voxel-Based
Clouds." *SIGGRAPH 2023, Advances in Real-Time Rendering in Games* course, 2023.** [talk] [recent]
<https://advances.realtimerendering.com/s2023/index.html> (Guerrilla's pages:
<https://www.guerrilla-games.com/read/nubis-cubed>, <https://www.guerrilla-games.com/read/nubis-evolved>;
2022: "Nubis, Evolved: Real-Time Volumetric Clouds for Skies, Environments, and VFX",
<https://advances.realtimerendering.com/s2022/index.html>; 2015: "The Real-time Volumetric Cloudscapes
of Horizon: Zero Dawn", <https://advances.realtimerendering.com/s2015/index.html>)

The three-talk arc that defined game clouds. 2015: a ray-marched 2.5D cloud layer from a weather map and
Perlin–Worley noise, Beer–Powder lighting, temporal reprojection. 2022: the layer becomes flyable 3D at
1080p without temporal upscaling and doubles as VFX (superstorms with internal lightning). 2023: voxel
clouds modelled with fluid simulation, ray-march acceleration through compressed signed distance fields,
up-rezzed dense voxel data, light-sampling acceleration and approximations for dark edges and inner glow
— shipped for flying mounts and time-of-day in Horizon Forbidden West's expansion.
*Bearing:* Forge needs 2015-style layer clouds first (weather, day/night, cheap) and the Nubis³ voxel
path later for flight; the SDF-accelerated march is the same trick the terrain SDF already uses.

**Sébastien Hillaire. "Physically Based Sky, Atmosphere and Cloud Rendering in Frostbite." *SIGGRAPH
2016, Physically Based Shading in Theory and Practice* course, 2016.** [talk] [still-current]
<https://blog.selfshadow.com/publications/s2016-shading-course/>

The production integration the 2020 paper later simplified: atmosphere LUTs, volumetric clouds and
their shadows, aerial perspective on everything, and — the part that matters for GI — how sky and clouds
feed the sun and sky lighting of the rest of the scene consistently, with energy conservation across the
pieces.
*Bearing:* the checklist for "integrated" (Bauer's word): one set of transmittances used by the sky,
the aerial perspective, the cloud shadows and the ambient term.

**Bartlomiej Wronski. "Volumetric Fog: Unified Compute Shader-Based Solution to Atmospheric
Scattering." *SIGGRAPH 2014, Advances in Real-Time Rendering in Games* course, 2014.** [talk]
[foundational]
<https://advances.realtimerendering.com/s2014/index.html> (author index:
<https://bartwronski.com/publications/>; generalisation: Hillaire, "Towards Unified and
Physically-Based Volumetric Lighting in Frostbite", Advances 2015,
<https://advances.realtimerendering.com/s2015/index.html>)

The froxel volume: a low-resolution view-aligned 3D texture into which density, scattering and lighting
from every light (with shadow maps) are injected per froxel, then integrated front-to-back in one pass to
give in-scattering and transmittance per pixel, temporally reprojected and applied to opaque and
transparent surfaces alike. Hillaire's 2015 talk generalises it — participating-media entities, phase
functions, sun shadows and local lights — and hooks it into the same aerial perspective.
*Bearing:* the fog Forge should build for near-field media, with the aerial-perspective LUT as the far
field; the same froxel grid later carries volumetric GI from the probes or cache.

**Henrik Wann Jensen, Frédo Durand, Michael M. Stark, Simon Premože, Julie Dorsey, Peter Shirley. "A
Physically-Based Night Sky Model." *SIGGRAPH 2001*, 2001.** [paper] [foundational]
<https://graphics.stanford.edu/~henrik/papers/nightsky/> (DOI 10.1145/383259.383306)

Everything that lights a night scene, with numbers: the Moon as a geometric body with measured
elevation and albedo maps and phase-dependent brightness, stars from a catalogue with position and
magnitude, zodiacal light, galactic background, airglow, and the same atmosphere scattering all of it.
Offline Monte Carlo, but the components and their radiances are what a real-time night sky must
reproduce at the right exposure.
*Bearing:* Forge's nights should come from this list — moonlight in lux by phase, a star catalogue
rendered as points at correct magnitudes, airglow as faint ambient — with the Hillaire LUTs evaluated
for the Moon as a second light. Aurora is left out (see below).

---

## 7. Reflections

**Tomasz Stachowiak. "Stochastic Screen-Space Reflections." *SIGGRAPH 2015, Advances in Real-Time
Rendering in Games* course, 2015.** [talk] [still-current]
<https://advances.realtimerendering.com/s2015/index.html>

SSR as importance sampling: per pixel sample the GGX lobe, march the hierarchical depth buffer, resolve
by reusing neighbours' hits with BRDF-weighted contributions, then accumulate temporally — glossy
reflections at any roughness with the right blur shape from one or two rays per pixel. Its failures
(off-screen, behind-object) are the ones RT reflections exist to fix.
*Bearing:* the raster tier's reflection method and the first-hit source even in RT tiers (screen
first, ray on miss); it also defines the resolve/denoise pattern reused for RT reflections and for
water.

**Johannes Deligiannis, Jan Schmid. "It Just Works: Ray-Traced Reflections in 'Battlefield V'." *GDC
2019*.** [talk] [still-current]
<https://www.gdcvault.com/play/1026282/It-Just-Works-Ray-Traced>

The first shipped hybrid: SSR where it works, rays where it does not, ray budget scaled by roughness and
importance, ray binning for coherence, shader generation to cover Frostbite's materials in hit shaders,
and a bespoke denoiser; candid about BVH build cost, skinned and destructible geometry, and the months
of tuning — hence the title.
*Bearing:* the middle tier's reflections. The costs it lists (material coverage in hit shaders, dynamic
BVH) are the ones Forge must pay for any RT tier, so pay them once, for reflections first.

---

## 8. Ambient occlusion and screen-space indirect

**Jorge Jimenez, Xian-Chun Wu, Angelo Pesce, Adrian Jarabo. "Practical Real-Time Strategies for
Accurate Indirect Occlusion." *SIGGRAPH 2016, Physically Based Shading in Theory and Practice* course,
2016.** [talk] [still-current]
<https://blog.selfshadow.com/publications/s2016-shading-course/> (Intel's implementation: XeGTAO, MIT,
archived April 2024, <https://github.com/GameTechDev/XeGTAO>)

GTAO: horizon-based AO derived from the cosine-weighted visibility integral so it matches ray-traced
ground truth, a fitted multi-bounce term that tints occlusion by albedo, bent normals, and specular
occlusion from the same data; well under a millisecond on 2016 consoles with temporal accumulation.
XeGTAO is the clean open implementation (depth MIP chain, Hilbert/R2 sampling, optional bent normals,
built-in denoise) — archived by Intel but complete.
*Bearing:* the AO for all raster tiers and the specular-occlusion source for probe-lit specular; port
XeGTAO's compute shader to Slang rather than rewrite it.

**Olivier Therrien, Yannick Levesque, Guillaume Gilet. "Screen Space Indirect Lighting with Visibility
Bitmask." *The Visual Computer*, 2023 (arXiv 2301.11376).** [paper] [recent]
<https://arxiv.org/abs/2301.11376>

Replace GTAO's two horizon angles with a bitmask of occupied sectors around the hemisphere, so surfaces
of assumed constant thickness occlude only their own sector and light passes behind thin objects; the
same bitmask yields one-bounce screen-space indirect light at GTAO cost. Currently the best
screen-space occlusion/indirect term and the basis of several engines' SSGI.
*Bearing:* the no-RT tier's near-field GI: SSILVB for the first metre, probes or cascades beyond —
the same split Lumen makes.

---

## 9. Upscaling, anti-aliasing, frame generation

**NVIDIA. "DLSS 4" (GeForce News, 6 January 2025) and "DLSS 4.5" (GeForce News, 6 January 2026).**
[web] [recent]
<https://www.nvidia.com/en-us/geforce/news/dlss4-multi-frame-generation-ai-innovations/> ·
<https://www.nvidia.com/en-us/geforce/news/dlss-4-5-dynamic-multi-frame-gen-6x-2nd-gen-transformer-super-res/>
(Ray Reconstruction origin, August 2023:
<https://www.nvidia.com/en-us/geforce/news/nvidia-dlss-3-5-ray-reconstruction/>)

DLSS 4 moved Super Resolution, DLAA and Ray Reconstruction to transformer models on all RTX GPUs and
introduced Multi Frame Generation (up to three generated frames per rendered frame) on RTX 50. DLSS 4.5
adds a second-generation SR transformer (about five times the compute of the first; RTX 20/30 lack FP8
and pay more for the larger models) and, on RTX 50 from 31 March 2026, Dynamic Multi Frame Generation up
to 6×. Ray Reconstruction (2023) is the AI denoiser that replaced NRD in Cyberpunk, Alan Wake 2 and
Doom. No "DLSS 5" has been announced as of this writing.
*Bearing:* target DLSS 4.5's SR/RR/MFG feature set through Streamline; RR changes the renderer
contract (it consumes noisy diffuse/specular radiance and guide buffers, not a denoised image), so
plan the RT tiers' outputs for it.

**AMD. "AMD FSR SDK 2.3" (FidelityFX SDK 2.x). GPUOpen, June 2026.** [code] [recent]
<https://gpuopen.com/amd-fidelityfx-sdk/> (Intel: XeSS SDK releases,
<https://github.com/intel/xess/releases>; XeSS 2.0.1 coverage,
<https://www.phoronix.com/news/Intel-XeSS-SDK-2.0.1>)

FSR 4 is AMD's ML upscaler (RDNA 4 acceleration, now also RX 7000, Shader Model 6.6), shipped as FSR
Upscaling 4.1.1 with Frame Generation 4.0.1 plus the "Redstone" ML set: Ray Regeneration 1.2.0 (a
standalone ML denoiser for RT — AMD's Ray Reconstruction) and Radiance Caching 0.9.0 (technical preview
of an online-trained GI cache — AMD's NRC). Distribution is signed DLLs behind
`amd_fidelityfx_loader.dll`; source is not published (a brief accidental publication in 2025 was
withdrawn, per press). FSR 4 is DirectX 12 only — no Vulkan path as of SDK 2.3 — and FSR 3.1 remains
the open-source, Vulkan-capable fallback. Intel's XeSS 2 (SDK 2.0.1, March 2025) added frame
generation, XeLL and Vulkan SR; 2.1 (July 2025) opened FG to non-Intel GPUs; XeSS 3 (March 2026) added
3×/4× multi-frame generation on Arc; binaries only.
*Bearing:* for a Vulkan engine the cross-vendor upscaler in 2026 is FSR 3.1 (open) or XeSS 2
(binary), not FSR 4; keep every upscaler behind one interface and add FSR 4 when AMD ships Vulkan.

**NVIDIA. "Streamline 2.14.1." GitHub, September 2026 (MIT).** [code] [recent]
<https://github.com/NVIDIA-RTX/Streamline> (DLSS inputs:
<https://github.com/NVIDIA-RTX/Streamline/blob/main/docs/ProgrammingGuideDLSS.md>; licence:
<https://github.com/NVIDIA-RTX/Streamline/blob/main/license.txt>)

The plugin framework Forge already uses: DLSS SR, DLSS-G/MFG, DLSS RR, Reflex and NIS behind one API on
D3D11/12 and Vulkan ≥ 1.2, MIT-licensed, binaries from the releases page since 2.7.32. The DLSS guide
fixes the contract: required colour, depth and motion-vector buffers; optional exposure (else
auto-exposure); jitter offsets in the per-frame constants; motion vectors in either [−1, 1] or pixel
units with an explicit scale; and resource-lifetime flags. RR additionally consumes noisy
diffuse/specular and guide buffers.
*Bearing:* every frame the renderer must produce sub-pixel jitter applied in the projection, dilated
motion vectors (including particles and, where needed, UI), unjittered depth, linear pre-exposed HDR
colour, an exposure value, and a reactive/transparency mask — the same set FSR, XeSS and TAA consume.

**Brian Karis. "High-Quality Temporal Supersampling." *SIGGRAPH 2014, Advances in Real-Time Rendering
in Games* course, 2014.** [talk] [foundational]
<https://advances.realtimerendering.com/s2014/index.html> (evolution: Jimenez, "Filmic SMAA: Sharp
Morphological and Temporal Antialiasing", Advances 2016,
<https://advances.realtimerendering.com/s2016/index.html>)

UE4's TAA and the vocabulary everyone still uses: jittered projection with a low-discrepancy sequence,
reprojection with the dilated velocity of the closest depth, history rectification by neighbourhood
clamping/clipping in YCoCg, luminance-adapted blend weights against flicker, a sharpening resolve.
Jimenez 2016 is the Activision counterpart with SMAA edge handling and "filmic" flicker control.
*Bearing:* Forge's TAA exists; these are the references to compare it against and the fallback for
GPUs without an ML upscaler.

**Lei Yang, Shiqiu Liu, Marco Salvi. "A Survey of Temporal Antialiasing Techniques." *Computer
Graphics Forum* (Eurographics State of the Art Reports) 39(2), 2020.** [paper] [still-current]
<https://leiy.cc/> (author index; DOI 10.1111/cgf.14018; practitioner guides: López, "Temporal AA and
the Quest for the Holy Trail", 2020, rev. January 2022,
<https://www.elopezr.com/temporal-aa-and-the-quest-for-the-holy-trail/>; Tardif, "Temporal Antialiasing
Starter Pack", <https://alextardif.com/TAA.html>)

The taxonomy: sampling and jitter, reprojection and velocity, history validation (clamping, clipping,
variance), the ghosting/blur/flicker triangle, and the relationship to upsampling and checkerboarding —
naming the failure modes before ML upscalers absorbed most of them. López's post is the best hands-on
walk through the same trade-offs; Tardif's is the minimal working implementation.
*Bearing:* the checklist for validating any temporal stage (TAA, upscaler, denoiser) in the golden-image
tests: disocclusion, thin geometry, transparency, camera cuts.

---

## 10. Colour pipeline, camera, validation

**Jorge Jimenez. "Next Generation Post Processing in Call of Duty: Advanced Warfare." *SIGGRAPH 2014,
Advances in Real-Time Rendering in Games* course, 2014.** [talk] [foundational]
<https://advances.realtimerendering.com/s2014/index.html>

The bloom everyone uses — a progressive downsample/upsample chain with 13-tap filters that is stable,
wide and cheap, with the firefly suppression HDR needs — plus scatter-as-gather motion blur, depth of
field, lens flares and film grain, and how they order and interact with exposure.
*Bearing:* copy the bloom chain; the rest of the post stack is taste and comes after exposure and the
display transform are right.

**Krzysztof Narkowicz. "Automatic Exposure." Blog, 9 January 2016.** [web] [still-current]
<https://knarkowicz.wordpress.com/2016/01/09/automatic-exposure/>

Histogram metering: build a log-luminance histogram on the GPU, discard the darkest and brightest
percentiles, take the average of the rest as the scene key, meter on illuminance rather than final
colour where possible, adapt with different rates for brightening and darkening, and give artists an
exposure-compensation curve over EV. Short, and what Frostbite, Unreal and most others converge on.
*Bearing:* the exposure Forge needs for a 0.001–100,000 lux world; combine with Lagarde's EV100 and the
pre-exposure the upscalers and denoisers consume.

**Hector Yee. "A Perceptual Metric for Production Testing." *Journal of Graphics Tools* 9(4), 2004
(tool: PerceptualDiff).** [paper] [foundational]
<https://pdiff.sourceforge.net/>

Compare two renders with a model of human vision (contrast sensitivity, luminance adaptation, spatial
frequency) rather than pixel error, so noise-level and platform differences do not fail a regression
test while real changes do; pdiff is the small tool that implements it and has been used for renderer
regression for twenty years.
*Bearing:* judge Forge's golden images by a perceptual metric with a tolerance per tier — the PT tier
against a converged reference, raster tiers against their own goldens — never by exact match.

---

## Recommendation for Forge

*Opinion, shaped by what is already built (Hillaire sky, CSM, TAA, Streamline DLSS) and by the two
target GPUs. Everything above is sourced; this is not.*

**Make hardware ray tracing the product's minimum.** id Software and MachineGames set the floor at RTX
2060 SUPER / RX 6600 in 2024–2025 and press reads GTA VI as RT GI on a PS5. The RTX 3080 is two
generations above that floor. A no-RT path still exists — for the editor on a laptop, for a headless
server that renders thumbnails, for a future handheld — but it is a tier, not the target, and it should
be the *same* GI design with a slower tracer (the Lumen lesson), never a second lighting system.

**The GI ladder, fallback to path tracing:**

| Tier | Hardware | Direct light | Indirect light | Shadows | Reflections | Denoise / upscale |
|------|----------|--------------|----------------|---------|-------------|-------------------|
| **T0 Raster** | any Vulkan 1.3 GPU | clustered forward+/visibility buffer | SSILVB near; world-space probe clipmaps (DDGI layout) updated by compute marches against the global SDF | CSM + PCSS; terrain shadow maps | stochastic SSR | TAA or FSR 3.1 |
| **T1 Hybrid** | RTX 20/30, RDNA 2/3 (the 3080) | clustered + ReSTIR DI for local lights | same probes updated by ray queries (DDGI proper); SHaRC optional | CSM far + RT near, adaptive samples | SSR first, rays on miss | NRD; DLSS SR / FSR 3.1 / XeSS |
| **T2 RT GI** | RTX 40/50, RDNA 4 | ReSTIR DI | ReSTIR GI + SHaRC (NRC on NVIDIA) | VSM or RT | RT, full-res | Ray Reconstruction or NRD; MFG |
| **T3 Path traced** | RTX 50-class | ReSTIR PT (Area ReSTIR later) | NRC, multi-bounce, caustics | RT | RT incl. particles | RR; 6× dynamic MFG |

T3 doubles as the *offline reference*: run it converged (no ReSTIR reuse, thousands of samples) and it
is the ground truth the golden-image tests compare T0–T2 against. Radiance cascades are the wildcard:
if the 2026 3D preprint holds up in a prototype, they replace the T0 probe update and possibly the T1
far field, because they are noise-free and cascade like the terrain.

**Adopt, not write:** Streamline (have it; MIT), NRD (MIT), the shader side of RTXDI (reservoir and
resampling headers — check its licence for redistribution), RTXGI 2's SHaRC and NRC (check licence),
NVIDIA's OMM SDK for alpha-tested foliage, XeGTAO ported to Slang, Bruneton's reference atmosphere and
pdiff as test oracles. **Write:** the probe clipmaps and their SDF/ray-query updaters, clustered
lighting, the visibility buffer and material classification, clouds (2015 layer first), froxel fog,
exposure and display transforms as data, and later VSM. **Do not adopt:** NVRHI or the Donut framework,
RTXPT as a codebase (D3D12-first; read it, do not vendor it), FSR 4 until it has a Vulkan path, neural
materials until the material library is frozen.

**Build order.** (1) Physical light units on every light, histogram exposure, AgX/ACES/PBR-Neutral as
data, and the perceptual golden-image harness — cheap and it unblocks every later comparison. (2) The
visibility buffer and clustered lighting, because every tier shades through them. (3) Probe clipmaps
with the ray-query updater (T1) and the SDF-march updater (T0), fed by the sky-view LUT for ambient.
(4) Hybrid reflections and RT sun shadows near the camera. (5) ReSTIR DI with NRD, then RR. (6) ReSTIR GI
plus SHaRC. (7) The path-traced tier from RTXDI's PT mode plus NRC, with cluster acceleration structures
for the procedural geometry. Clouds, froxel fog and the night sky run in parallel on the atmosphere
track, since they gate the look more than GI does (Bauer, Patry).

**The demo that proves it: "Harbour, dusk to night, storm coming."** One procedurally generated
coastal town from one seed: twenty thousand emissive windows, street lamps and boat lights; wet streets
and an ocean; a cloud front crossing during a 90-second dusk-to-night with the moon rising; the same
seed rendered on all four tiers with side-by-side capture and the converged reference. Pass criteria:
T2 within the perceptual tolerance of the reference; T1 at 60 fps, 1440p, DLSS Quality on the RTX 3080;
T3 at 60 fps, 4K, DLSS Performance plus MFG on the 5070 Ti; no visible seam when the camera crosses from
street to orbit. Because lighting is a pure function of seed, time and weather state, co-op clients agree
on it by construction — only the weather state needs replicating.

---

## Checked and left out

Kept, because a bibliography that lists only what it found is not auditable.

- **Terrain horizon and shadow maps for large worlds.** The right references are Timonen and
  Westerholm, "Scalable Height Field Self-Shadowing", *Computer Graphics Forum* 29(2) (Eurographics
  2010), and Max, "Horizon Mapping: Shadows for Bump-Mapped Surfaces", *The Visual Computer* 4(2),
  1988. The author page (wili.cc) presents a certificate for another host, Wiley returned 403, the EG
  digital library refused connections and Springer redirected to a login. Both are real; neither could
  be confirmed here. Etienne, "Large Scale Terrain Rendering in Call of Duty" (SIGGRAPH 2023 Advances)
  is confirmed on the course index but I could not verify that it covers terrain shadows.
- **Aurora.** Baranoski, Rokne, Shirley, Trondsen, Bastos, "Simulating the Aurora Borealis" (Pacific
  Graphics 2000), "Simulating the Aurora" (*JVCA* 2003) and "Simulating the Dynamics of Auroral
  Phenomena" (*ACM TOG* 2005), and Lawlor and Genetti, "Interactive Volume Rendering Aurora on the GPU"
  (*JGT* 2011): IEEE Xplore and Semantic Scholar returned empty pages, Wiley blocked, the Waterloo group
  pages 404'd, and the UAF PDF downloaded but could not be text-parsed. Real, unverified, no entry.
- **Weather rendering as a sub-topic** (rain, snow, wetness, lightning) was not researched for this
  file beyond the cloud and fog entries; it needs its own pass.
- **"Rendering the World of Red Dead Redemption 2"** — no such talk. The real Rockstar talk is Bauer,
  SIGGRAPH 2019 Advances (§3). No GDC or SIGGRAPH talk by Rockstar on GI, GTA V or GTA VI was found.
- **Cyberpunk 2077 GDC 2024 talk** ("RT: Overdrive in Cyberpunk 2077 Ultimate Edition") — listed on
  NVIDIA On-Demand, but the page rendered only navigation; cited in §2 as existing, not summarised.
- **"An Introduction to Neural Shading"** (SIGGRAPH 2025 Courses, ACM DOI 10.1145/3721241.3733999) —
  ACM DL blocked; not cited.
- **Silvennoinen, "Large-Scale Global Illumination at Activision"** and **Halen et al., "Global
  Illumination Based on Surfels"** (both SIGGRAPH 2021 Advances) — confirmed on the course index but
  the slides were not fetched, so no summary is offered; both are leads for the T1/T2 tiers, the surfel
  one especially.
- **DLSS 5** — nothing announced by NVIDIA as of September 2026; DLSS 4.5 is current.
- **Which shipped games use NRC versus SHaRC** — could not be verified; not claimed.
- **FSR 4 "accidental" source release** and **"trailer 2 captured on PS5"** — press only.
- **Ray Tracing Gems II** (2021) chapter details — not fetched; mentioned as existing only.
- **Vulkan ray tracing final specification** (Khronos blog, 23 November 2020: `VK_KHR_acceleration_structure`,
  `VK_KHR_ray_tracing_pipeline`, `VK_KHR_ray_query`, `VK_KHR_pipeline_library`,
  `VK_KHR_deferred_host_operations`) — confirmed, but a spec announcement rather than a technique, so
  recorded here instead of as an entry.

**One correction worth carrying.** The brief dated "Temporal AA and the quest for the holy trail" to
2020; the post's page shows a January 2022 revision date and the author as Emilio López (redorav).
Cited under 2020, rev. 2022.

---

## Verification notes

- Every entry above was checked against at least one page that answered on 2026-09-23. Blocked or
  empty: ACM DL, Wiley, JCGT HTML pages (PDFs did download), EG digital library (connection refused),
  IEEE Xplore, Semantic Scholar, Springer (login redirect), casual-effects.com (404), research.nvidia.com
  for the 2020 ReSTIR and 2020 TAA-survey pages (404; author pages used instead), Eurogamer, NeoGAF,
  Notebookcheck, PCGamingWiki, and the Steam store page (age gate; the Steam `appdetails` API was used).
- PDFs that were reachable but that the fetcher could not text-parse, so bibliographic data comes from
  the linking page or the canonical record: Fernando 2005 (PCSS), Olsson et al. 2012, Burns and Hunt
  2013, Lawlor and Genetti 2011.
- DOIs seen on a fetched page: Area ReSTIR (10.1145/3658210), Zeltner et al. (10.1145/3659577),
  Hillaire 2020 (via the EG bitstream path `cgf14050`), Yang et al. (via the diglib path `cgf14018`).
  The other DOIs given (Bitterli 2020, Ouyang 2021, Lin 2022, Müller 2021, Crassin 2011, Olsson 2012,
  Bruneton 2008, Jensen 2001) are the canonical records for those papers and were not read off a
  fetched page; re-check before quoting them in a paper.
- Talk titles for the Advances courses (2014, 2015, 2016, 2019, 2021, 2022, 2023) and the SIGGRAPH
  Physically Based Shading courses (2016, 2020) were read from the course index pages, which list
  speakers and affiliations; the slide decks themselves were not fetched, and the summaries rely on
  the talks as known.
- Intel's GitHub release pages print month and day without a year; years were fixed from the Phoronix
  article dating XeSS SDK 2.0.1 to 17 March 2025, giving 2.1.0 July 2025, 2.1.1 November 2025, 3.0.0
  March 2026, 3.0.2 July 2026.
- The web-search budget for this session ran out midway; the remaining verification used direct fetches
  of known URLs, which is why a few author-page and press fallbacks appear where a search would have
  found a better mirror.
- Rockstar statements: the only primary Rockstar page cited is rockstargames.com/VI (release date
  19 November 2026). Everything about GTA VI rendering is press.

## Implementation notes from Forge (2026-09-24, issue #7, D-022)

Build-order step (1) of the recommendation, minus the perceptual metric, in the ballad and
the bench. What the implementation taught:

- **Space breaks a physical sky.** With the sun at 128 000 lux the rocks meter at EV100 ≈
  15, where a real starfield (about 10⁻⁴ cd/m²) is eight orders of magnitude below black.
  The sun's disc can be physical (its illuminance over its solid angle, 1.9 · 10⁹ cd/m²) and
  the planet can be lit like the rocks, but stars and nebula have to be authored in units
  of the sunlit surface and documented as art direction, or they disappear exactly as they
  do in Apollo photographs.
- **AgX flattens a mostly dark scene.** Its log encoding spends the display range on 16.5
  stops, so a sky of dim nebula lifts to a flat grey; ACES's toe keeps space black. The
  choice is per scene, which is the point of shipping the curves as data: the ballad
  defaults to ACES, the engine and the bench to AgX. ACES here is Hill's fit of the 1.x
  RRT + ODT; the 2.0 output transform is still to do.
- **Pre-exposure needs the history rescaled.** A temporal filter stores pre-exposed
  colour; multiplying the fetched history by the ratio of this frame's exposure to the
  previous one keeps adaptation free of ghosts at no cost.
- **Where the histogram lives matters more than how it is built.** Atomics from 5 600
  workgroups into host-visible memory: in video memory through Resizable BAR the GPU is fast
  but the CPU pays 0.02 ms to read 1 KB back; in cached system memory the CPU is fast and
  the GPU pays 0.4 ms of PCIe atomics. Counting in device-local memory and copying 1 KB to
  a cached buffer costs 0.02 ms in total.
- **A fence wait does not make device writes visible to the host.** The readback needs a
  barrier to `HOST_READ`; the render graph gained a `HostRead` buffer access for it
  (issue #21 extends it to the older statistics and capture readbacks).
- **Metering in space.** Dropping the black bin and keeping the 50th–98th percentiles
  lets the sunlit rocks dominate the key inside the belt (in open space the nebula takes
  over and the image opens by half a stop): along the 90-second path EV100 stays within
  14.4–15.0 and reverses by more than 0.1 EV once every nine seconds.

## Implementation notes from Forge: the atmosphere (2026-09-24, issue #8, D-023)

Hillaire 2020's transmittance and multiple-scattering tables, lifted from the `world`
project onto the render graph, with the per-pixel march for a planet seen from space in
the ballad. What the port taught:

- **From space the air is a hairline.** An Earth-sized planet seen at 18° of angular
  radius is 14 000 km away: its 100 km of atmosphere is 0.3° (three pixels at 1600×900),
  and the part that scatters much less than one pixel. The blue limb, the haze towards it
  and the red ring of sunset light round the night side are all there, sub-pixel, and
  TAA's jitter integrates them. At 50° (1 900 km up) the same air reads as a band. How big
  the planet looks is an art-direction choice, not a lighting one.
- **Distances of 20 000 km need care in f32.** Ray–sphere spans computed as `b² − c` lose
  kilometres at grazing rays; from the point of closest approach, as `(r − h)(r + h)`, they
  keep metres. Marching from the atmosphere's entry point keeps the samples precise.
- **Put the samples where the air is.** A grazing ray is 2 000 km long, but the air that
  matters sits within a few hundred kilometres of its lowest point; segments packed
  quadratically towards that point (and towards the ground for rays that hit it) make 16
  segments enough: within 4/255 of 128. Uniform segments would need several times more.
- **The sun stays white behind the limb.** Its disc is 10⁹ cd/m²; a transmittance of 10⁻²
  still clips at an exposure set for sunlit rock, as it does in a camera. The reddening
  shows in the air around it.
- **Cost follows coverage.** A cone test against the planet's direction skips the march for
  every other pixel (+0.01 ms with the planet out of view); in view the ballad's planet adds
  0.04–0.06 ms, a planet filling the screen 0.2 ms. Because the camera barely moves
  relative to the planet, a view-direction table around it (#26) would make that a lookup.

## Implementation notes from Forge: DLSS (2026-09-24, issue #8, D-024)

DLSS Super Resolution through Streamline 2.14, optional behind the `dlss` feature, with TAA
as the default. What the port taught:

- **Tag lifetimes decide copies.** Tagged `eOnlyValidNow`, as the previous project did,
  Streamline copies every input before evaluating: extra transfers, and transfer usage the
  render graph had not declared (validation errors). `eValidUntilPresent` is true here,
  since nothing writes the inputs between the DLSS pass and the present, and DLSS then reads
  them in place.
- **Third-party passes need an honest declaration.** NGX clears its output at the transfer
  stage before its compute shaders write it. A storage-write declaration left that clear
  unsynchronised (a write-after-write hazard across frames). The graph gained a `Custom`
  access that names the layout, stages and access bits, and the barrier is derived as usual.
- **The interposer changes presentation.** With Streamline's `vkQueuePresentKHR` in the
  path, MAILBOX was held to the display's refresh in most runs (the acquire waited 8 ms);
  IMMEDIATE was not. Streamline also needs Vulkan 1.3's `privateData` feature enabled.
- **LOD in output pixels.** With a cluster DAG the geometric detail follows the render size
  unless the LOD error is scaled: in render pixels, Quality and Performance drew faceted
  rocks that no upscaler can restore. Measured in output pixels, every mode draws the same
  0.56 M triangles, and DLSS only reconstructs shading.
- **DLSS costs what the output costs.** 0.43–0.46 ms at 1600×900 in every mode, against
  0.05 ms for the TAA resolve. Upscaling pays only when the scene saves more than that at
  the input size: not the 0.33 ms ballad, but the million-instance city at 1440p.
- **Pre-exposure.** Passing the frame's pre-exposure (relative to a fixed EV100) kept the
  output at the input's level. DLSS undoes it on the way out, so its history can follow the
  automatic exposure the way the TAA's rescaled history does.

## Implementation notes from Forge: sky light and GTAO (2026-09-25, issues #47, #48)

- **The sky-view table is already an environment map.** Hillaire's table covers the whole
  sphere around the camera, with the planet's sunlit ground below the horizon. Nine SH
  coefficients of it, convolved with the cosine (Ramamoorthi and Hanrahan), give a
  surface's sky light for 0.016 ms, ground bounce included. Under the default sun a roof gets
  0.075 of the sun and a wall about 0.20, most of it from the ground.
- **GTAO's noise has to follow the TAA's.** XeGTAO cycles its noise over 64 frames. Forge's
  TAA jitters over 8, and its history then drifted from pattern to pattern: 0.24 % of a
  static view's pixels changed by more than two levels over 32 frames, against 0.09 %
  without AO. Repeating the noise every 8 frames brought it to 0.10 %; a second denoise
  pass did nothing for it.
- **Screen-space AO is contact AO at city scale.** A 1.5 m radius (2.2 m with XeGTAO's
  multiplier) reads recesses, basins and the feet of walls. It does not reach the sky
  hidden by a street's buildings, which is the probes' work (step 3), or rays against the
  shadows' TLAS.

## Implementation notes from Forge: DDGI probes (2026-09-25, issue #53, D-036)

- **Written from the papers.** The 2019 and 2021 JCGT papers give the whole algorithm. The
  RTXGI-DDGI SDK is under the NVIDIA RTX SDKs License: free and royalty-free, but ported
  source keeps NVIDIA's notice and terms. It served as a reference for constants.
- **Cost is the fixed rays, not the shaded ones.** At first every probe traced its 32
  relocation rays to 60 km each frame, and the ray pass took 0.69 ms. Capped at 3.5 cells and
  left unshaded, it took 0.44 ms. With probes that settle, stop tracing them, and turn off
  inside buildings or in open air, it takes 0.37–0.40 ms. About a fifth of each cascade
  lights something in the city.
- **Relocation must stop.** Probes between two surfaces (pushed off the ground into what reads
  as the inside of a ledge, moved back through it) changed state every frame. Every change
  restarted their maps, and a mirror window showed it as drift (0.61 % of pixels over 32
  frames). A probe now settles after 8 updates, which the 2021 paper's relocation at
  initialisation already suggests.
- **Rotations on TAA's cycle.** A new random rotation of the rays every frame kept the maps
  wandering (0.27 % slow change on a static view). Repeating 8 rotations with TAA's jitter
  halves it, as it did for GTAO's noise.
- **Negative remainders.** On the RTX 5070 Ti's driver, a signed remainder of a negative
  number (SPIR-V `OpSRem`) came out as the unsigned one. A ring buffer indexed by world
  cells hits this at the first negative cell; take remainders of positive numbers.
- **The lookup's cost is in the resolve.** It is 0.46 ms of the 1.05 ms at 1440p, more than
  the probes' own passes. Removing its integer divisions barely moved it. The resolve's
  registers are the likelier cause, as they were for the mirror rays (#52), so a pass of its
  own is next.
