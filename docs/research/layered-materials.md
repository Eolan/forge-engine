# Layered materials at city scale (2026-11-01, D-094)

**Why:** an authored city's surfaces are often not one set of textures each but a stack of tiled
layers: a base (concrete, steel, paint) and up to twenty more (grime, rust, peeling paint, wet
patches), each a tiling material template with its own maps, tinted from a palette, with its
roughness and metalness remapped, and blended over the ones below by its own mask, a microblend
sharpening the mask's edge. Shipped open-world games author their cities this way, and an authored
city loaded into Forge brings such materials. The look lives in those layers (their colours, their
tiling detail, their edges), and drawing them must fit D-094's frame: 120 fps at 2560 × 1440 through
DLSS or FSR. This note compares the ways a real-time engine can draw them and what each costs. Forge's own
material model so far is one row a section (`RenderLayer`), the ground's layer map (#42) and the
world-grid decals in the resolve (D-074); `research/vegetation-materials.md` recommends "a small
stack of `(layer, blend)` pairs", after Unreal's Material Layers.

Found by a research agent over the web on 2026-11-01 (sources at the foot). The estimates marked so
are the agent's arithmetic, not measurements.

## What shipped engines do

- **REDengine 4's "multilayered" materials** (the RED modding wiki's pages on the shader and its
  properties; a CD Projekt RED technical artist in Adobe's Substance magazine): up to 20 layers,
  each a template with a tile factor (`matTile`), a palette colour (`colorScale`), roughness and
  metalness levels in and out, a normal strength and an opacity, and a microblend (texture, tile,
  normal strength, contrast) that cross-fades the layer's mask with the microblend's alpha where the
  mask is not fully white. Shader parameters `MaskAtlas`, `MaskTiles` and `LayersStartIndex` point to
  masks kept in an atlas; the technical artist describes each pixel having to "compute and flatten its
  layers", the engine using "compute shaders to bake-in visible layers into a runtime texture", and
  masks stored as "compressed data tiles packed based on their surface coverage area": a mask atlas
  and a runtime composited texture (family b below). No timings published.
- **Unreal Engine 5:** Material Layers evaluate every layer per pixel (a forum post seen only in a
  search result: the mask re-sampled per layer). Substrate's slabs cost about N times the lighting;
  parameter blending folds them into one slab (28 bytes a pixel against 108; the default budget 80).
  **Runtime Virtual Texturing** caches shading, but projected from above along a volume's −Z, so not
  for walls; at most 4,096 tiles; 8 uploads a frame in game by default; block compression cuts its
  memory 4–8×; an extreme negative mip bias makes its pool thrash.
- **Frostbite (Battlefield 3, Widmark 2012):** splatting the terrain's layers on screen cost 10–20
  ms (a search excerpt of the slides); its cached procedural virtual texture composited tiles in
  0.2–0.5 ms a frame on PS3, plus 2.5–3 ms writing the G-buffer.
- **Far Cry 4 (Chen, GDC 2015, adaptive virtual textures):** 64 m sectors each with its own virtual
  image, near ones 64K pixels (about 10 texels/cm), far ones 1K; about 512K² virtual with about
  9K² resident; pages 264² with a 4-pixel border, 8× anisotropy with software trilinear; terrain
  decals baked into the pages; the work spread over frames, coarse levels first; fast vehicles churn
  the cache.
- **Call of Duty (Activision, the terrain talk, 2021–2023):** one alpha mask per layer (up to 32) gave
  high memory and an unpredictable shader cost; replaced by a one-byte layer index a texel plus a tint
  map, at most 4 samples a pixel. Its virtual texture: 256² pages, a 512² indirection, a 240 × 136
  feedback buffer read back three frames later, requests capped at a third of the pages, pages kept
  three frames at least, a missing page composited at low resolution first and rebuilt after 15 and
  30 frames.
- **The Order: 1886** capped blending at run time at 4 layers (offline compositing had no limit);
  **Gears of War 4** cooked the static layers offline and kept only the live-blended ones at run
  time. id Tech 5's page pool, table, feedback and coarser fallback: abstracts only.
- **Texture-space shading** (NVIDIA's Turing blog, AMD's texel shading) moves lighting into texture
  space: more than this needs.

## The three families for Forge

**(a) The layers blended per pixel in the resolve.** Each active layer costs about four fetches
(albedo, normal, roughness and metalness, microblend); twenty raw masks are twenty more fetches just
to find which layers are zero. At DLSS Quality's 1707 × 960 (1.64 Mpx), all twenty layers without an
early-out are about 100 fetches a pixel, 164 M a frame: 0.5 ms at the 5070 Ti's peak, 2–4 ms in
practice (estimate). Four layers with compacted masks: about 18 fetches a pixel, 0.2–0.6 ms
(estimate). The levers:
- **the masks compacted offline** into each texel's top four (layer, weight) pairs, as Call of
  Duty's index map;
- **an early-out** on a weight under a small threshold;
- **per 8 × 8 tile, a loop over the union of its pixels' layers,** so a wave runs one loop (no
  assumption on the wave's width).

It works with DLSS's jitter as it is; FSR's mip bias (−1.58 at Quality) raises the texture
bandwidth.

**(b) A runtime composited cache.** RVT's top-down projection does not suit buildings: Forge would
need a virtual image per mesh asset in its masks' UV space (the masks are uniquely unwrapped), shared
by the instances of a mesh and setup. Resident texels about 2–4× the display's pixels: an 8K² pool
at 2–3 bytes a texel (BC1/BC7 + BC5 + BC4), 128–192 MB. Feedback must take the upscaler's mip bias
(or the cache resolves blurry) and hysteresis (the jitter makes requests noisy). A 264² page of four
layers is about 1.4 M fetches plus the block compression: 32 new pages a frame cost about family
(a) at four layers. It wins where many layers overlap or the camera is slow, loses driving fast.
Pages do not shimmer, but refinement pops: the fallback must be a coarser level of the same content.

**(c) Baking offline.** A 20 × 10 m façade at 1,024 texels/m is 200 M texels, about 0.5 GB
compressed: unique full-detail bakes are out. A 2K bake loses about ten times the tiling detail up
close. The hybrid works: bake the low frequencies (tint, roughness, the large normals) and keep one
or two detail layers live (The Order, Gears 4). Bakes suit far levels of detail, where tiling
repeats alias.

**Detail at 2560 × 1440:** at a 90° horizontal field of view a pixel covers 0.78 mm at 1 m: a texel a
pixel is about 1,300 texels/m at 1 m, 260 at 5 m, 26 at 50 m. A 2K template repeating every 2 m
(about 1,024 texels/m) uses its top level only within about 1.3 m.

## Patents

Status and expiry as Google Patents estimates them, not legal conclusions.

| Patent | Holder | Covers | Priority | Status, expiry | Alternative |
|---|---|---|---|---|---|
| US9881391B2 (CN105684037B; EP3053144A2 ceased) | Microsoft | a texel shader run in the sampling thread on a cache miss, its result cached | 2013-10-02 | active, ~2034-02-07 | feedback-driven compositing in a separate pass (Frostbite 2012, id Tech 2009 before it); never composite inline on a miss |
| US9728002B2 (US only) | AMD | shading only the texels not already cached from another frame | 2015-12-18 | active, ~2035-12-18 | the nearest to (b); Far Cry 4 (GDC, March 2015), Frostbite and id predate it: **flag before building (b)** |
| US12444121B2 | AMD/ATI | chaining screen→shade-space and shade-space→material gradients for anisotropic filtering in decoupled shading | 2023-06-30 | active, ~2043-11-09 | composite pages at each texel's own isotropic footprint, anisotropy only when sampling the cache |
| US10699427B2 | NVIDIA | a hardware texture-footprint query | 2018-07-03 | active, ~2038-07-03 | feedback from derivatives in the shader |
| US10388058B2 (WO2018151870A1) | Microsoft | sampling a residency (MinMip) map | 2017-02-16 | active, ~2037-09-15 | a software indirection texture |
| US12430838B2 | Intel | reading the sampler feedback map | 2021-11-03 | active, ~2044-01-31 | a classic feedback buffer read back by compute or the CPU |
| US9024959 | Intel | a page miss driving a tile's decode, then the sample re-issued | 2009-12-21 | active, ~2031-10-19 | feedback-driven residency, no page faults |
| US12347016B2 | Tencent | virtual texture coordinates computed in the vertex shader | 2021-03-19 | active, ~2043-05-27 | coordinates in the resolve (Forge's shape already) |
| US7295212 | SGI | blending textures with a mip-mapped mask | 2002-04-30 | expired 2022 | — |

None found for Ubisoft's adaptive virtual texture, Epic's RVT, REDengine's multilayered system or
blending layered materials per pixel.

## Recommendation for Forge

1. **Build (a) first:** one layer-evaluation function in the resolve, the masks compacted offline
   to each texel's top four layers, the early-out under a threshold, the loop over each tile's
   union of layers. Family (a) meets no patent found.
2. **The uncapped path (every layer, every mask) as the reference image,** within the same
   function.
3. **Measure on fixed camera paths** through an authored city: the resolve's GPU time at 1707 × 960
   (a budget of about 1–1.5 ms), a heat map of active layers a pixel, Nsight's texture throughput and
   L1 hit rate, and ꟻLIP of the four-layer cap against the reference at a fixed exposure.
4. **Only if it misses the budget,** the same function as the compositor of a mesh-UV cache (b):
   feedback-driven, 128² + 4 pages, block-compressed by compute, about a 160 MB pool; the owner sees
   AMD's US9728002 and US12444121 first.
5. **Bakes (c) for the far levels of detail only.**

## Sources

wiki.redmodding.org (cyberpunk-2077-modding: for-mod-creators-theory/materials/multilayered,
developers/shaders/multilayered, .../configuring-materials/multilayered-material-properties);
adobe.com/products/substance3d/magazine/cyberpunk-2077-a-world-full-of-substance.html;
gdcvault.com/play/1021761 (Adaptive Virtual Texture Rendering in Far Cry 4) and
game.watch.impress.co.jp/docs/news/20150308_691757.html; illu.tistory.com/1463; gcores.com/articles/191717;
research.activision.com/publications/2021/09/boots-on-the-ground--the-terrain-of-call-of-duty;
therealmjp.github.io (siggraph-follow-up; bindless-texturing-for-deferred-rendering-and-decals);
history.siggraph.org/?p=84229; dev.epicgames.com (Substrate overview; Runtime Virtual Texturing;
virtual texturing settings); developer.nvidia.com/blog/texture-space-shading/;
gpuopen.com (FSR's upscaler manual); patents.google.com: US9881391, US9728002, US12444121B2,
US10699427, US10388058, US12430838B2, US9024959, US7295212, US12347016B2. The 5070 Ti's 280 texture
units and 686.6 GTexel/s from search results only; two EA and Activision slide PDFs did not parse.
