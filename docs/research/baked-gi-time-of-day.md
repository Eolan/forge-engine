# Baked GI that follows the time of day: precomputed transfer, relit probes, sparse bricks

*Research note, 2026-11-01, a research agent (WebSearch and WebFetch only, no browser). Not legal
advice: patent statuses and expiry dates are Google Patents' estimates ("an assumption, not a legal
conclusion"); Google Patents rate-limited the second half of the session, so some records were seen
only through search engines. Facts are marked *read* (fetched today; slides read as text),
*search-recorded* (a search engine's summary or a secondary write-up) or *from memory*.*

## Why

The owner wants a rasterised quality mode whose lighting is precomputed ("a good technology to
have, it avoids a lot of real-time computations") while sun, moon, day, night and weather still
change at run time. D-094 holds it to a shipped open-world city game's raster benchmark: 152.3 fps
mean, 6.48 ms p50, DLSS Quality at 2560 × 1440 on the RTX 5070 Ti. That game carries about 22,000
precomputed GI files, one per 64 m cell, in a sparse "brick" structure (~12 GB, ≈ 0.55 MB a cell),
reflection probes with per-face depth and a sky-visibility flag, and a list of far lights. Its
developers call the raster GI bounce light from sky, sun and local lights that reacts to changes:
that matches the relit-transfer family (§1c), not baked hours (§1d). An inference, not a fact.

## Bottom line

- **Build first: relit transfer probes in sparse bricks** (The Division's design, §1c), baked by
  Forge's path tracer, one file per 64 m cell. Each frame the GPU lights a few thousand surface
  samples ("surfels") with the live sun, moon, sky and lamps, averages them into bricks, and sums
  bricks and sky into the probes. Nothing is keyed by hour: weather, night and lamps cost no data.
- **Keyed bakes** (AC4 to AC Shadows, §1d) are simpler but scale badly: Ubisoft puts a uniform AC
  Shadows bake at 624 days and ~1.9 TB, and its shipped sparse bake still needs 33 keys and has no
  local-light bounce. **Enlighten** (§2) needs lightmap UVs on target meshes and a licence.
- **Reflection probes:** G-buffer cubes (albedo, normal, depth, sky mask) relit for the hour and
  normalised by the local irradiance (§4).
- **Patents (§5):** Microsoft's PRT core is expired; nothing was found on Enlighten's runtime, on
  Far Cry 3's or The Division's relit probes, or on brick volumes. **One live family needs the
  owner's decision: Imagination's US 10,672,183, US 11,127,198 and GB 2571656 (to 2036) may read on
  the per-frame relight.** Avoided by design: ARM's US 10,672,177 (a bake guided by form factors),
  Microsoft's US 7,948,490 (transfer from depth buffers over many directions, to 2028) and Unity's
  US 12,260,527 (neural denoising of probe data).

## 1. Precomputed radiance transfer on probes and volumes

**a. Sloan, Kautz and Snyder, PRT (SIGGRAPH 2002)** (*from memory*). Per vertex of a rigid object,
a transfer vector (diffuse, 25 floats a channel) or a 25 × 25 matrix (glossy) maps distant lighting
in order-5 SH to exit radiance, self-shadowing and interreflection included; shading is a dot or
matrix product with the live lighting. Distant, low-frequency light only; no local lights; static
geometry. Follow-ups: clustered PCA (2003), local deformable PRT (2005), Ambient Dice (2017).

**b. Far Cry 3, "Deferred Radiance Transfer Volumes" (Gilabert and Stefanov, GDC 2012)**
(*search-recorded*; slides behind GDC Vault's login). Sparse probes, each a transfer matrix from SH
lighting to irradiance in a 4-vector basis plus sky visibility, in 64 × 64 m sectors of about 70
probes. Relit on the CPU each frame into volume textures applied in screen space; handles time of
day and local lights (muzzle flashes, explosions). Chosen as open-world lightmaps cost too much.

**c. The Division (Stefanov, GDC 2016)** (*read*). Manhattan, over 6 km², day-night cycle,
dynamic weather, dense interiors.
- **Bake:** probes on the hits of a 4 m raycast grid (ground, along walls). A G-buffer cube per
  probe gives the surfels it sees, clustered on the CPU in two hash grids: 1 m cells by position and
  principal normal, then 4 m irradiance "bricks". 5–6 s a sector.
- **Per probe:** position, sky visibility (a spherical shadow term, "assume sky visible in all other
  directions"), a range of brick factors (brick index, basis weights). Basis: HL2 ambient cube.
- **Sectors:** a 2D grid of 64 m cells, at most 1,000 probes (typically 200–300), streamed.
  Manhattan: 1.07 GB on disk (≈ 272 KB a sector), 3,932 sectors, 1.16 M probes, 56.4 M surfels.
- **Each frame (GPU, async on consoles):** light each surfel (the sun through the shadow map,
  noting surfels it misses; local lights meeting the sector, static ones cached) and average them
  into bricks; probe = sky colour × sky visibility + Σ brick radiance × weight; write a
  100 × 50 × 100 m irradiance volume (32 × 16 × 32 voxels a direction). Bounces: each surfel reads
  its closest probe from the last frame. Weather: a per-surfel snow weight whitens the albedo.
- **Cost:** two sectors a frame (600–800 probes): Xbox One 0.95 ms (not async), GTX 760 0.47 ms.
- **Around it:** an interior volume chosen by stencil against bleeding; a far-field 2D texture of
  sky-only "sector probes"; probe sky visibility, SSAO and baked AO; dynamic objects only receive.
- **Post-mortem:** dark spots where probes were missing, a sharp indoor/outdoor edge, coarse
  multi-bounce ("causes dark spots"), resolution bound by bake time and compression.

**d. Keyed bakes: Assassin's Creed** (*read*: Wronski, GDC 2014; Ubisoft, SIGGRAPH 2025 Advances).
AC4's "Deferred Normalized Irradiance Probes" bake the sun's bounce on the GPU at 8 hours a day, as
8-bit normalised irradiance in the FC3 basis on one 2 m layer, de-normalised and blended at run time
with live sky and AO. Havana: 30,000 probes, 600 KB of VRAM, 1.2 ms on PS3, 8 minutes on a GTX 680.
Ubisoft's 2025 table:

| Game | Area | Probe distance | Keys | Bake | Disk |
|---|---|---|---|---|---|
| Unity | 4 km² | 0.5 m | 4 | 4 days | 15 GB |
| Syndicate | 6 km² | 1 m | 9 | 3 days | 9 GB |
| Origins, if uniform | 256 km² | 1 m | 11 | 156 days | 468 GB |
| Shadows, if uniform | 256 km² | 1 m | 44 | 624 days | ~1.9 TB |
| Shadows, shipped sparse | 256 km² | 0.5–2 m | 33 (11 hours × 3 seasons) | 5 days | 10 GB |

Origins introduced "sparse GI", Valhalla multi-state environments. Shadows merged spring and summer
to fit, keeps the baked diffuse with "relightable local cubemaps" on its lower modes, and hopes to
"get rid of baked technique". Keys are exact at each hour; a lamp or a fire can only be scaled.

**e. Silvennoinen and Lehtinen (SIGGRAPH Asia 2017)** (*search-recorded*): a few radiance probes
rendered live, and a precomputed local operator (SVD-compressed) rebuilding each receiver from
nearby probes; 3.9 ms with fully dynamic lights, no dynamic geometry, glossy approximate.

## 2. Precomputed form factors with run-time light injection

**Enlighten** (Geomerics; ARM from 2013, *search-recorded*; Silicon Studio later, *from memory*;
*read*: Martin and Einarsson, SIGGRAPH 2010).
- **Precompute:** the scene is cut into "systems", each a small lightmap on a hand-parameterised
  low-poly target mesh (UVs projected onto the detail mesh) plus form factors between clusters.
  Geometry only: 10–30 minutes a level, distributed.
- **Run time (CPU, asynchronous):** point-sampled direct light of any type is mapped to lightmap or
  probe output; the previous output feeds back, one bounce a frame, converging "in 4 frames or
  less". The sun has a fast path with precomputed visibility to static geometry; sun motion is a
  global update staggered over frames. Frostbite 2 updated about one system per core per frame.
- **Limits:** static geometry (dynamic objects receive through probes), latency (visible on muzzle
  flashes), target meshes to author, a commercial licence. Unity's "Realtime GI" is Enlighten, and
  Unity 6 is its last supported release (*read*, Unity's blog). ARM claimed 25 km² at 30 fps (2016).
- **Frostbite's static GI** (O'Donnell, GDC 2018, *read*): the Flux path tracer (CPU, Embree)
  bakes L1 SH lightmaps (indirect only) and irradiance volumes with sky visibility; ">200 CPU
  core-hours" for some scenes; no time of day.

## 3. Sparse brick and probe-volume structures

- **Unity Adaptive Probe Volumes** (*read*, Unity 6 manual and blog): bricks of 4 × 4 × 4 probes,
  spacing 1, 3, 9 or 27 m, denser near geometry; pixels read their 8 nearest probes; cells stream
  disk → CPU → GPU. About 16 B a probe at L1, 32 B at L2 (a forum account, *search-recorded*).
  - **Lighting Scenarios:** baked sets switched or blended on shared probe positions.
  - **Sky occlusion:** one static SH value a probe ("the amount of indirect light the probe receives
    from the sky, including light that bounced off static GameObjects") times the live ambient
    probe: one bake, a day-night cycle for sky light only; surfaces bounce as grey.
  - **Leaks:** virtual offset, dilation, adjustment volumes, up to 4 rendering-layer masks.
- **Unreal's volumetric lightmaps** (*search-recorded*): bricks of 4 × 4 × 4 SH cells, densest near
  static geometry (200 cm by default); Paragon's Monolith 5 MB → 30 MB; static only. **Quantum
  Break** (SIGGRAPH 2015, *search-recorded*): an adaptive octree of irradiance volumes.
- **Baked DDGI-like probes:** octahedral irradiance plus depth, filled offline. The depth test
  fights leaks at tens of times L1 SH's memory (*from memory*: ~1.7 KB a probe against 16–48 B);
  the light stays fixed unless relit as in §1c.

## 4. Baked reflection probes relit for the time of day

- **Relightable cubes,** captured unlit (albedo, normal, depth, sky mask) and relit each frame or
  round robin with the live sun, sky and GI, then filtered: Far Cry 4 (McAuley, GDC 2015,
  *search-recorded*: relit every frame from the lighting environment and sky dome, the 16 × 16 mip
  as the diffuse cube); Project A1 (Unreal Summit 2016, *read*: sun and SH2 sky plus the probe GI's
  average colour and directional shadowing); AC Shadows (*read*: "relightable local cubemaps" and a
  128 × 128 global cube updated one face a frame). Unity lists it as "under consideration".
- **Normalised probes:** divide the cube by its irradiance at capture, multiply by the receiver's
  from the volume (Lazarov, SIGGRAPH 2013, *from memory*; Unity APV's normalisation, *read*).
- **Per-face depth and a sky flag** (the shipped game's probes) give parallax correction and
  occlusion, and let the live sky and clouds show where a texel saw sky.

## 5. Patents

Searched on Google Patents by assignee (Geomerics, Silicon Studio, Ubisoft, Electronic Arts) and
by number (*read* unless marked). Unity and Activision by web search only; Epic not checked.

| Technique | Patent | Holder | Where | Status (Google) | Alternative |
|---|---|---|---|---|---|
| PRT core | US 7,262,770 B2; continuation US 7,609,265 B2 | Microsoft | US | expired, fees | it is the alternative |
| PRT variants | US 7,061,489 B2 (lapsed 2018); US 7,633,503 B2 (lifetime, 2026-04-08); US 7,663,623 B2 (fees); US 7,167,176 B2, US 7,884,820 B2 (*search-recorded*); US 2005/0083340 A1 abandoned | Microsoft | US | expired | — |
| PRT bake on the GPU | US 7,948,490 B2: transfer from depth buffers rendered over sampled directions | Microsoft | US | **live, to 2028-03-30** | bake by ray tracing |
| Bake final gather | US 10,672,177 B2, GB 2543562 B: rays distributed by directional irradiance from form factors | ARM (ex-Geomerics) | US, GB | **live, to 2036-10-18**; 4th-year fee paid 2023-11-21 | no form factors in the bake |
| Enlighten runtime | none: Geomerics' 7 records date from 2014 on (shadows, curves, primitives); Silicon Studio's 7 JP records are not on lighting | — | — | — | Martin 2010, published |
| Relit and keyed probes | none under Ubisoft (28 hits on lighting terms, none on GI) | — | — | — | published 2012–2016 |
| Probe culling by zones | US 10,137,372 and family (`lighting-patents.md`) | Square Enix | US, EP, JP, CA | live to 2036–2037 | region numbers, not masks |
| Probe update from rays | US 10,672,183, US 11,127,198, GB 2571656 and family (`lighting-patents.md`) | Imagination | US, GB; DE pending | **live, to 2036** | below |
| DDGI for RT reflections | US 2023/0316629 A1 | Electronic Arts | US | application, not checked | not a bake |
| Probe denoising | US 12,260,527 B2 (noisy probe data into a neural network); continuation 19/061,781; EP 4116934 | Unity | US, EP | granted 2025-03-25 (*search-recorded*, claims not read) | more samples, a non-neural filter |
| DDGI | US 2021/0012562 A1 | NVIDIA | US | abandoned 2023 | — |

**Imagination and the relight step** (claims as read in `lighting-patents.md`, 2026-10-17). US
10,672,183 and GB 2571656 update a probe's directional representation "from a previous frame" with
rays traced from it; US 11,127,198 from "a lighting contribution for each of one or more rays". A
Division-style relight sums brick radiances weighted by factors from rays traced at bake time. Is
that such a contribution, and does recomputing each probe from scratch (no hysteresis) stay outside
"from a previous frame"? For counsel. Prior art before the 2015-03-03 priority: Far Cry 3 (2012),
Enlighten (2010), Sloan (2002), Greger (1998); The Division (2016) came after. Pure PRT (FC3: a
matrix times sun and sky SH) or keyed bakes avoid the question but lose local-light bounce.

## 6. For Forge

**Build first: relit transfer probes in sparse bricks, one file per 64 m cell** (§1c): sun, moon,
sky, weather and lamps all change without a rebake, and no ray is traced at run time.

**The bake (Forge's GPU path tracer), per cell:**
1. **Probes:** a sparse brick tree, 4 × 4 × 4 probes a brick at 1, 3 or 9 m (1 m only near
   geometry); D-078's room grids indoors; a region number per probe (D-068).
2. **Rays,** about 1,024 a probe: hits become surfels in the two hash grids (1 m by position and
   principal normal; 4 m bricks); misses give the probe's sky visibility in L1 SH.
3. **Surfels:** albedo from D-095's layered materials (folded), emissive, a snow or wet weight.
4. **Option, sun-path visibility per surfel:** 64 directions over the sun's and moon's yearly band
   as 64 bits, so far cells light without a shadow map (which missed surfels in The Division).
5. **Lights:** the cell's lamp list (indices into the world's lights) and a far-field record (a
   sky-only "sector probe" and aggregated far lights, like the shipped game's list).

**Stored per cell** (estimate, 1,500 probes, 30,000 surfels):

| Item | Size |
|---|---|
| Probes: position 6 B, L1 sky visibility 4 B, factor range 4 B | ~21 KB |
| Factors: ~24 bricks a probe × (2 B index + 4 B L1 weights) | ~216 KB |
| Surfels: position 6 B, normal 2 B, albedo 3 B, flags 1 B, sun-path bits 8 B (option) | 360–600 KB |
| Bricks, lights, far field | a few KB |

So 0.6–0.85 MB a cell (the shipped game ≈ 0.55 MB, The Division ≈ 272 KB). Reflection cubes come
on top, a few a cell: BC albedo, octahedral normal, 16-bit depth, a sky bit (~0.35 MB at 128²,
0.1 MB at 64²), relit one face a frame, normalised by the volume.

**Run time** (passes `gi/surfel_light`, `gi/brick`, `gi/probe`, `gi/volume`, in the F1 overlay):
relight the 3 × 3 cells round the camera each frame, farther cells staggered (the sun moves slowly),
into a camera clipmap of L1 SH or bricks behind an indirection texture (as APV); shade per pixel.

**Budget (estimate, to measure).** The Division relit 600–800 probes in 0.47 ms on a GTX 760; nine
cells of 1,500 probes is ~19 times the work. The 5070 Ti has ~19 times the FP32 rate and ~5 times
the bandwidth: about 0.5 ms if compute-bound, up to 2 ms if bandwidth-bound, plus 0.1–0.3 ms of
sampling at 1707 × 960, inside a frame that must beat 6.48 ms p50.

**Steps:** (1) one cell of the private district baked and relit against Forge's path-traced
reference at fixed exposure, at noon, dusk and night with lamps; (2) streaming and the far field;
(3) reflection cubes; (4) wet and snow on the surfels.

**Open questions:**
- **Imagination (§5):** the owner decides, or counsel reads, before the relight is built. Fallback:
  FC3-style pure transfer plus a per-probe local-light term.
- **Doors, destruction, moved props:** the transfer is static. Valhalla-style multi-state cells, or
  local probes made dynamic?
- **Basis:** L1 SH (Frostbite, Unity) or the ambient cube (The Division); how many factors a probe?
- **Leaks:** is D-068's region number enough at doorways (a number, never a mask: Square Enix), or
  are DDGI-like depth tests worth tens of times the memory?
- **Bounces:** closest-probe feedback gave dark spots; a baked second bounce per surfel instead?
- **Disk:** 22,000 cells at ~0.7 MB is ~15 GB (quantised weights, BC, LZ to try); sun-path bits
  or the shadow map trade memory against coverage.

## Sources

- Sloan, Kautz, Snyder, SIGGRAPH 2002 (*from memory*).
- Far Cry 3: https://gdcvault.com/play/1015326/Deferred-Radiance-Transfer-Volumes-Global ; https://cedec.cesa.or.jp/2012/program/AB/C12_I0287.html
- The Division (slides read): http://mrakobes.com/Nikolay.Stefanov.GDC.2016.pdf ; https://gdcvault.com/play/1023273/Global-Illumination-in-Tom-Clancy
- AC4: https://bartwronski.com/wp-content/uploads/2014/03/ac4_gdc.pdf (also GPU Pro 6, 2016).
- AC Shadows: https://www.realtimerendering.com/advances/s2025/content/Advances%202025%20-%20Raytracing%20the%20world%20of%20Assassin's%20Creed%20Shadows.pdf ;
  https://www.ubisoft.com/en-us/game/assassins-creed/news/4XbPPtFyQEtIMWrA9xVDmZ/assassins-creed-shadows-tech-qa
- Silvennoinen and Lehtinen 2017: https://arisilvennoinen.github.io/Projects/RTGI/index.html
- Enlighten: https://www.advances.realtimerendering.com/s2010/Martin-Einarsson-RadiosityArchitecture(SIGGRAPH%202010%20Advanced%20RealTime%20Rendering%20Course).pdf ;
  https://www.arm.com/company/news/2016/03/real-time-global-illumination-reaches-open-world-gaming
- Frostbite: https://media.contentapi.ea.com/content/dam/eacom/frostbite/files/gdc2018-precomputedgiobalilluminationinfrostbite.pdf
- Unity: https://docs.unity.com/en-us/engine/6000.0/manual/lighting-overview/lighting/probevolumes/concept ;
  https://docs.unity.com/en-us/engine/6000.3/manual/lighting-overview/lighting/probevolumes/probe-volumes-change-lighting-at-runtime/skyocclusion ;
  https://unity.com/blog/engine-platform/new-ways-of-applying-global-illumination-in-unity-6 ;
  https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/probevolumes-options-override-reference.html ;
  https://activation.unity3d.com/roadmap/1888-reflection-probes-relighting
- Unreal: https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-lightmaps-in-unreal-engine
- Quantum Break: https://www.advances.realtimerendering.com/s2015/SIGGRAPH_2015_Remedy_Notes.pdf
- Far Cry 4: https://gdcvault.com/play/1022234/Rendering-the-World-of-Far ; https://cedec.cesa.or.jp/2015/session/ENG/14698.html
- Project A1: https://www.slideshare.net/KiHyunwoo/unreal-summit-2016-seoul-lighting-the-planetary-world-of-project-a1
- Patents, *read* on patents.google.com/patent/NUMBER/en: US7262770B2, US7609265B2, US7061489B2,
  US7948490B2, US7663623B2, US20060214931A1 (US7633503B2), US20050083340A1, US10672177B2,
  US10665010B2, GB2543562B, GB2551388B, GB2555797A, US10297075B2; assignee queries on
  patents.google.com/xhr/query (Geomerics, Silicon Studio, Ubisoft, Electronic Arts).
  *Search-recorded:* US7167176B2, US7884820B2, US12260527B2, Activision's US10282894
  (participating media, not GI). Imagination, Square Enix, NVIDIA: `lighting-patents.md`.
