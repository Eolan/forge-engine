# A cache on the surfaces for D-088's bounce: techniques and patents

*Research note, 2026-10-30, a research agent (WebSearch and WebFetch only). Not legal advice:
patent statuses and expiry dates are Google Patents' estimates, which it labels "an
assumption, not a legal conclusion"; Google Patents rate-limited the second half of the
session, so the claims of US 11,315,310, US 12,394,144, US 12,456,261 and the Tencent family
were not read (abstracts and publications only).*

## Why

D-088's traced bounce lights each ray's hit with the probes for the bounces after. Against
Forge's own path (`r.gi.reference=4`) the probes are biased bright in enclosed, high-albedo
spaces: a probe in free space sees more than a surface does, and the bounce loop compounds it.
The village's whitewashed room reads 1.4–1.7 with the probes alone and 1.1–1.4 with the bounce.
A cache whose records sit on the surfaces, their rays leaving the surface's own side, read at
the hits, would take that bias out.

## Bottom line

1. **No surface radiance cache read at ray hits is clearly free of live patents.**
   - NVIDIA's Pantaleoni family (US 11,244,493, US 11,961,176, US 12,633,037; to about mid/late
     2039): independent claims that, read literally, cover reusing radiance samples from
     secondary rays across surface points, possibly hit-distance-guided denoising too.
   - NVIDIA's US 9,953,457 (path space filtering, to 2034-12-15) is also broad.
   - Ward 1988, Jensen 1996 and Keller 2013 are strong prior art against them, never tested in
     court. A counsel opinion is needed before shipping any design below.
2. **The narrower live patents can be designed around:** hash tables with collision resolution
   (US 10,614,613, to 2038), ranked choice of where to update and a switch to the cache by a
   ray's length (US 11,790,594, to about 2041, a continuation pending), neural caches (NRC
   family), world-space ReSTIR (AMD US 11,816,781; NVIDIA US 11,315,310), Gautron's hash-map
   mechanics (US 11,609,899 family, US 12,394,144, US 11,925,860, US 12,456,261), anisotropic
   irradiance records (Adobe, to about 2028), probes gathering from surfels (Tencent).
3. **No patents found** for Epic (Lumen), EA (GIBS surfels), Unity (Surface Cache GI) or Embark
   (kajiya). NVIDIA's DDGI application US 2021/0012562 A1 is "Not Active" on Google Patents
   (Forge's probes).
4. **Recommended design:** persistent surface records on a dense camera-centred clipmap, no
   hashing (Design A below), estimated 0.5–0.9 ms at 1600 × 900 on the RTX 5070 Ti (to measure).

## The techniques

### Ward's irradiance caching (1988) and its GPU forms

Records at surface points hold the irradiance of a hemisphere of rays from that point; a record
is valid within a radius from the harmonic mean of its rays' distances (it shrinks near walls),
and rejected for points whose normal differs or that lie in front of it. Gautron et al. (EGSR
2005) splatted the records' footprints with blending ([EGSR 2005](https://diglib.eg.org/items/3717df14-843b-4d34-811d-135bbc164e69);
[Křivánek & Gautron 2009](https://link.springer.com/10.1007/978-3-031-79540-4)); Bunnell's
GPU Gems 2 discs iterate for multi-bounce ([ch. 14](https://developer.nvidia.com/gpugems/gpugems2/part-ii-shading-lighting-and-shadows/chapter-14-dynamic-ambient-occlusion-and));
Coombe et al. kept radiosity in texels ([GI 2004](https://graphicsinterface.org/?p=4777)).
Patents: anything filed before about 2006 has expired. Adobe's US 8,207,968 (anisotropic,
elliptical influence areas, filed 2008-11-26, [uspto.report](https://uspto.report/patent/grant/8207968))
and US 8,223,148 (cosine-weighted sample points near edges, filed 2008-06-19,
[Google Patents](https://patents.google.com/patent/US8223148)) run to about 2028: no elliptical
footprints until then.

### Path space filtering (Keller et al. 2013/14) and its jittered spatial hashing (Binder, Fricke, Keller 2018/19)

Averaging nearby path vertices' contributions; the hashed form quantises a vertex's features
with jitter into a key and accumulates into a hash table ([arXiv 1902.05942](https://arxiv.org/abs/1902.05942)).
NVIDIA's US 9,953,457 B2 (priority 2013-04-22, to 2034-12-15): weighted averaging of nearby
vertices' contributions times their throughput ([Google Patents](https://patents.google.com/patent/US9953457)).
US 10,614,613 B2 (priority 2017-07-28, to 2038-07-25; CN 110766778 B granted, DE pending): claim 1
quantises a vertex's features, hashes them, resolves collisions in a hash table, accumulates
with a counter and averages ([Google Patents](https://patents.google.com/patent/US10614613)). It
reads directly on SHaRC and GI-1.0's caches; a dense array index without collision resolution
does not meet its words literally (how far "hash" stretches is uncertain).

### Gautron's spatial hashing (2020, 2022)

World-space AO in a multiresolution hash, extended to diffuse light in 2022
([2020](https://history.siggraph.org/?p=41377), [2022](https://history.siggraph.org/?p=128425));
SHaRC descends from it. NVIDIA patents (all about 2040–2043): concurrent hash-map updates
(US 11,609,899 B2, US 12,013,844 B2, US 12,360,985 B2, US 2025/0378068 A1 pending,
[Google Patents](https://patents.google.com/patent/US12013844B2/en)); projective hash maps
(US 11,925,860 B2, [Google Patents](https://patents.google.com/patent/US11925860B2/en)); one
consistent representative point per cell (US 12,394,144 B2, [Google Patents](https://patents.google.com/patent/US12394144));
hash cells' boundaries reshaped by periodic functions (US 2023/0298274 A1, apparently granted as
US 12,456,261, [nweon](https://patent.nweon.com/30508)).

### NVIDIA SHaRC (RTXGI 2.x)

A world-space hash grid keyed by 17-bit cell coordinates, a level by distance and three normal
sign bits ([HashGridCommon.h](https://raw.githubusercontent.com/NVIDIA-RTX/SHARC/main/include/HashGridCommon.h));
a path tracer on about 4 % of the pixels updates it, a resolve pass accumulates and evicts, and a
path stops on a cache hit unless its segment is shorter than a voxel
([Integration.md](https://github.com/NVIDIA-RTX/SHARC/blob/main/docs/Integration.md)). Surfaces of
the same normal on both sides of a wall share a cell once the voxel is wider than the wall. 40 B
an entry. Licence: NVIDIA RTX SDKs License ([License.md](https://raw.githubusercontent.com/NVIDIA-RTX/SHARC/main/License.md)):
object code in an application only, no open-source licence, no patent grant; its source in
Forge's public repository would not fit those terms (as AMD's FSR SDK stays local). Patents:
US 10,614,613 and the Pantaleoni family, unlicensed by it.

### NVIDIA's neural radiance cache (Müller et al. 2021)

An MLP trained online from short paths, queried at path vertices, about 2.6 ms at 1080p in the
paper ([arXiv 2106.12372](https://arxiv.org/abs/2106.12372)); in practice tied to NVIDIA's tensor
hardware ([RTXGI](https://github.com/NVIDIA-RTX/RTXGI)). US 11,610,360 B2 (to 2041-12-03) and its
siblings claim a neural cache throughout: avoided by not using one.

### AMD's GI-1.0, Brixelizer GI, world-space ReSTIR

GI-1.0 (Boissé et al.): screen probes plus a world cache of outgoing radiance at secondary
vertices, keyed by position and ray direction (no normal), two hashes with linear probing, an
exponential average and radiance feedback; weak indoors when the bounce's source is off screen
([arXiv 2310.19855](https://ar5iv.labs.arxiv.org/html/2310.19855)). 1.9–3.4 ms in total at 1080p
on an RTX 3080. Capsaicin and Brixelizer GI are MIT ([Capsaicin](https://github.com/GPUOpen-LibrariesAndSDKs/Capsaicin);
[Brixelizer](https://gpuopen.com/fidelityfx-brixelizer/)), no patent grant. AMD's US 11,816,781 B2
(world-space reservoir reuse by spatial hashing, to 2041-06-22, [Google Patents](https://patents.google.com/patent/US11816781B2/en));
NVIDIA's US 11,315,310 B2 (ReSTIR with lookups in a GI structure, [Google Patents](https://patents.google.com/patent/US11315310)).

### Epic's Lumen surface cache

Offline-placed cards per mesh, material captured into an atlas, light computed at card texels
and gathered from the cache itself for multi-bounce, updates under a budget; missing coverage
"only result[s] in lost energy, instead of leaking"; walls of at least 10 cm advised
([Lumen technical details](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine);
[SIGGRAPH 2022](https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf)).
No Epic patent on global illumination found on Google Patents.

### EA SEED's GIBS (surfels, 2021 and 2024)

Surfels spawned from the G-buffer at the least-covered pixel of each 16 × 16 tile, indexed in a
uniform grid near the camera and trapezoidal slices further out; each holds irradiance, a 4 × 4
hemisphere depth mean and mean square (Chebyshev visibility against leaks) and a ray-guiding map;
rays allocated by variance; ray hits read surfels for multi-bounce
([2021 slides](https://www.advances.realtimerendering.com/s2021/SIGGRAPH%20Advances%202021%20-%20Surfel%20GI.pdf);
[2024 deck](https://advances.realtimerendering.com/s2024/content/EA-GIBS2/Apers_Advances-s2024_Shipping-Dynamic-GI.pdf)).
About 5.4 ms for the whole GI on PS5. No EA patent on it found. Its variance-driven ray
allocation is near US 11,790,594's ranking, its ray guiding near US 12,067,667.

### Embark's kajiya irradiance cache

Twelve camera-aligned 32³ clipmaps with a dense indirection array (no hashing), at most 65,536
entries; a trace origin voted per voxel each frame; four ReSTIR candidates an entry a frame,
stored as L1 SH irradiance; read only at secondary hits. About 0.55 ms for about 128k rays and
384k shadow rays. Thin surfaces seen from both sides still leak; voting and deallocation flicker
([gi-overview](https://github.com/EmbarkStudios/kajiya/blob/main/docs/gi-overview.md)). MIT or
Apache-2.0, archived 2026-06-01. No patents found.

### Bevy Solari's world cache (Bevy 0.17 to 0.20, 2025–26)

Read after the note, on the owner's choice (2026-10-30), at Bevy 0.20.1
(`crates/bevy_solari/src/realtime/world_cache_*.wesl`).
- **The cells:** keyed by a hash of the quantised position and normal, with a checksum and up to
  three steps of linear probing on a collision, 2²⁰ of them. A cell lives ten frames unread, and
  the live ones are compacted by a prefix sum each frame.
- **The update:** each live cell gets its direct light by RIS over pre-sampled light tiles, and
  one cosine ray whose hit reads the cache (the bounces after) or the sky. A blend whose sample
  count is cut back where the luminance changes.
- **Cell size:** 15 cm, doubling with the distance from the camera, the level chosen
  stochastically and the query jittered in the tangent plane.
- **The reads:** at a path's hits, only when the ray is longer than √3 cells.

MIT or Apache-2.0; Apache-2.0 grants its contributors' own patents (section 3), not NVIDIA's.
- **US 10,614,613:** its index is that claim's quantise, hash and resolve.
- **US 11,790,594:** its read by the ray's length is near claim 16.

Taken with kajiya's dense index in place of the hash and an unconditional read: D-088's
decision.

### Unity's Surface Cache GI (preview, 2026)

(Position, normal) to patches, rays averaged per patch, "heavily inspired by" GIBS, kajiya and
SHaRC ([preview](https://discussions.unity.com/t/surface-cache-gi-preview/1720494)). No patents
found.

### Texture-space caches (Tatzgern et al., HPG 2024) and NVIDIA's geometry-mapped caches

Radiance cached in texture space at primary and secondary hits ([slides](https://highperformancegraphics.org/slides24/hpg24_oscgi.pdf)).
NVIDIA's US 11,790,594 B2 (Wright, Boksansky; to about 2041-10-17; continuation
US 2024/0029338 A1 pending, [Google Patents](https://patents.google.com/patent/US11790594)):
claim 1 ranks locations and casts rays from the higher ranked, claim 10 the same for groups of
geometry, claim 16 uses a hit's outgoing irradiance cache "based on the distance" between the two
locations.

## The patents

| Patent / application | Holder | Status (Google) | Est. expiry | What its claims cover | For Forge |
|---|---|---|---|---|---|
| US 9,953,457 B2 | NVIDIA | Active | 2034-12-15 | Weighted averaging of nearby path vertices' contributions | Broad; hard to avoid literally |
| US 10,614,613 B2 | NVIDIA | Active | 2038-07-25 | Quantise, hash, resolve collisions, accumulate, count, average | No hash table: a dense clipmap |
| US 11,244,493 B2, US 11,961,176 B2, US 12,633,037 B2 | NVIDIA (Pantaleoni) | Active | 2039 | Radiance at a point from other secondary rays' samples; cached samples within a region | Very broad; counsel needed |
| US 12,067,667 B2 (+ continuation pending) | NVIDIA | Active | about 2040–41 | Directional radiance choosing rays' directions (path guiding) | No guiding from the cache |
| US 11,790,594 B2 (+ US 2024/0029338 A1) | NVIDIA | Active / pending | 2041-10-17 | Ranked choice of where to update; using a cache by a ray's length | No ranking; read unconditionally |
| US 2021/0012562 A1 (DDGI) | NVIDIA | Not active | — | Probe DDGI | Forge's probes |
| US 11,610,360 B2 and siblings | NVIDIA | Active | 2041-12-03 | A neural cache trained online | No neural cache |
| US 11,609,899 B2 family | NVIDIA (Gautron) | Active / pending | about 2040 | Chained per-entry changes, one thread commits, coarser levels | Not that update scheme |
| US 11,925,860 B2 | NVIDIA | Active | 2042-09-08 | Points projected onto a 2D surface to key a hash | Avoid |
| US 12,394,144 B2 | NVIDIA (Gautron, Waechter) | Active | about 2041 | One consistent representative point per hash cell | Persistent records, no elected anchors |
| US 2023/0298274 A1 / US 12,456,261 | NVIDIA | Granted (unverified) | about 2043 | Cells' boundaries reshaped by periodic functions | Avoid |
| US 11,816,781 B2 | AMD | Active | 2041-06-22 | World-space reservoir reuse by spatial hashing | No world-space ReSTIR |
| US 11,315,310 B2 (+ US 2022/0198746 A1) | NVIDIA | Active | about 2041 | ReSTIR with lookups in a GI structure | No cache-fed ReSTIR |
| US 8,207,968 B2 / US 8,223,148 B1 | Adobe | Unknown | about 2028 | Anisotropic records; cosine-weighted sample points | Isotropic radius only |
| CN 115830208 B (+ US, EP) | Tencent | Active | about 2043 | Probes taking light from surfels within a distance | Probes never gather from records |

## Recommendation for Forge

**Design A: persistent surface records on a dense clipmap.**
- **Records:** anchor point and normal fixed for life, Ward's radius from the harmonic mean of
  its rays' distances, irradiance (fp16, optionally L1 SH), mean and variance, a 4 × 4 hemisphere
  depth mean and mean square, age, last use: about 128 B, 64k records about 8 MB. Rays leave from
  the anchor into its normal's hemisphere.
- **Spawning:** at the bounce's hits and the G-buffer where coverage is low, the least-covered
  pixel of each 16 × 16 tile (GIBS); anchors never move (no per-frame voting, no elected point
  per cell).
- **Index:** about 8 camera-centred levels of 32³ cells, the finest 10–15 cm; records inserted in
  every cell their radius overlaps by count, prefix sum and scatter. Dense indices, no hash, no
  collision resolution.
- **Validity:** accept a record at a hit only when the normals agree (dot > 0.8) and the hit lies
  within about 0.1 R of its tangent plane; weight by distance and by Chebyshev visibility from
  its depth moments. Opposite faces of a wall never mix.
- **Update:** every live record a fixed 2–4 cosine rays a frame from a rotated low-discrepancy
  sequence; each hit lit by the sun, the lamps and its albedo times the records there (feedback,
  infinite bounces). No ranking, no guiding.
- **Accumulation:** a variance-adaptive exponential average floored at about 1/64; new records
  ramp in, unused ones fade out before they are recycled.
- **Reads:** at every bounce hit, unconditionally (no switch on the ray's length); the pixels
  never read records directly; the probes stay for volumes, particles, movers and the first
  frames before a record exists, and never gather from records.
- **Cost (to measure):** 30–60k records × 4 rays and a shadow ray each, 360k lookups:
  0.5–0.9 ms estimated on the RTX 5070 Ti.
- **Patents:** every element traces to work before 2018 or to GIBS (2021) and avoids the narrow
  claims found; it still sits under the Pantaleoni and Keller 2013 claims.

**Design B, for generated architecture:** a cache texture per face of the generators' walls,
floors and ceilings (about 25 cm texels, in the spirit of Lumen's cards and Coombe 2004), read at
a hit by instance, face and local UV: no leak by construction. Only if Design A's thin-wall tests
fail.

**Avoid:** hash-grid caches (SHaRC, GI-1.0's world cache), neural caches, world-space or
cache-fed ReSTIR, Gautron's hash mechanics, anisotropic records, ray guiding from cached light,
and SHaRC's source in the public repository.

**Before building:** the owner's decision on the live patents, and a counsel opinion on
US 11,244,493, US 11,961,176, US 12,633,037, US 9,953,457 and US 11,790,594 (the agent's advice).

**The owner's decision (2026-10-30, D-088):**
- **The design:** Bevy Solari's world cache with kajiya's dense clipmap index, read at every
  hit.
- **The risk accepted without a counsel's opinion:** the Pantaleoni family and US 9,953,457.
- **Designed around:** US 10,614,613 (no hash) and US 11,790,594 (no switch by a ray's length).
- **Sources:** Bevy (MIT or Apache-2.0) and Embark's kajiya (MIT or Apache-2.0), both credited.

## Checked and left out

Take-Two's US 12,263,404 (a terrain bounce texture), Dassault's US 10,249,077 (triangles as
virtual point lights) and Thomson's US 9,082,230 (2010, light reflected towards a point) were set
aside as unrelated. Tatzgern et al.'s timings could not be extracted.
