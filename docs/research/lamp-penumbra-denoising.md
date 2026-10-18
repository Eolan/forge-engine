# Denoising the lamps' penumbrae (D-065 follow-up, #270)

This note covers D-065's step "a denoiser for the lamps' penumbrae". It builds on
[shadow-denoising.md](shadow-denoising.md) (#172, the sun) and
[interior-lighting.md](interior-lighting.md). A research agent wrote it on 2026-10-18 with the
sources checked that day.

**How each claim was checked:**
- **[read]**: the page or file was opened and read.
- **[local]**: read in Forge's own NRD checkout (`nrd-sdk/src`, NRD v4.18.0, git-ignored).
- **[search]**: only a search-result summary was seen.
- **[recalled]**: from knowledge of the paper; the PDF could not be re-read (over 10 MB, or the
  host blocks bots).
- **[inferred]**: reasoning or an estimate.

> **In short.**
> 1. The stair-steps come from each pixel seeing only 8 disc points (16 with two rays), turned by
>    its own fixed angle. TAA cannot average across pixels, so a wide penumbra settles into 8
>    dithered bands.
> 2. The simplest cure that avoids known patents is three old techniques plus the ratio estimator
>    (no patent found on any of them):
>    - interleave the lamp's sample points across a 4×4 pixel block and the 8-frame cycle (128
>      distinct points a lamp);
>    - filter the per-pixel ratio "shadowed ÷ unshadowed lamp light" with a fixed 4×4
>      geometry-aware box before shading;
>    - multiply the result into the exact, unshadowed lamp light.
> 3. It costs about 0.1–0.2 ms at 1600×900 on top of today's rays [inferred]. It needs no hit
>    distance, no variance, no history and no NRD.
> 4. NRD's SIGMA can take several lights in one pass through its translucency variant fed with the
>    same ratio. NVIDIA itself calls that "limited and hard to use", and it needs closest-hit rays.
>    Keep it as the upgrade if wide penumbrae still crawl in motion.
> 5. NVIDIA's shadow-denoising patents are broader than `shadow-denoising.md` said:
>    US 10,740,954 B2's claims 10 and 15 do **not** require an anisotropic kernel. Every filter
>    whose width comes from the occluder's distance is in their territory, SIGMA-like filters
>    included, until about 2039.

## 0. Why TAA leaves stair-steps (Forge's code, [inferred])

**What the code does now.** `lamps_at` (`shaders/meshlet.slang`):
- takes the three strongest lamps, plus one picked by its light;
- traces each ray to `sun_disc_point(pixel, noise_frame)`: 8 Vogel points, one a frame of the
  cycle, turned by an angle fixed per pixel;
- gives the strongest lamp a second ray half a cycle later.

**Why it fails.** Each pixel converges to its own 8-point (or 16-point) estimate, so visibility is
quantised to 1/8. Neighbouring pixels use different turns, so the bands become a static dither.
TAA's 3×3 clamp box on binary samples spans [0, 1], so it holds nothing in motion.

**Example, a campfire.** The fire's light is 12 cm in radius. A rock 0.5 m from it shades the
ground 2 m from the fire:
- the penumbra is about 0.24 × 1.5 / 0.5 ≈ 0.7 m wide;
- seen from 5 m at 1600 px over 90°, that is about 110 px;
- so it shows 8 bands of about 14 px each.

**What is needed.** About 100 or more distinct points integrated a pixel. That only happens if
neighbours' samples are combined: a spatial filter.

## 1. NVIDIA NRD's SIGMA and local lights

**Version facts [read, local].**
- v4.18.0 is a pre-release published 2026-10-09; it is what Forge pins (D-049).
- The last stable release is v4.17.3 (2026-04-30).
- 4.18 changed SIGMA's spatial filter from 8 to 12 taps and added
  `SIGMA_PRESERVE_SHADOWS_IN_PENUMBRA`.
- Since v4.2, one NRD instance can hold repeated denoisers, for example two SIGMAs (`UPDATE.md`).

**Local lights are supported [local].**
- The README lists "local light source (omni, spot)".
- `NRD.hlsli` has a local overload, `SIGMA_FrontEnd_PackPenumbra(distanceToOccluder,
  distanceToLight, lightSize)`:
  - penumbra = lightSize × d_occ / max(d_light − d_occ, ε), × 0.5 for the radius;
  - `lightSize` is the light's projection on the plane across the light's direction: 2r for a
    sphere.
- The infinite-light overload uses `tanOfLightAngularRadius = lightRadius / distanceToLight`.
- `SigmaSettings::lightDirection` is "needed only for directional light sources". At zero,
  `SIGMA_Blur` uses an isotropic tangent-plane kernel. A TODO in the code says anisotropy for
  other light types is not done.
- Kernel limits: `SIGMA_MAX_PIXEL_RADIUS` 32 px, at most 7 stabilised frames.

**Ray rules [local].**
- Rays go from the surface to the light.
- `ACCEPT_FIRST_HIT_AND_END_SEARCH` is ruled out: it gives a wrong, long hit distance.
- The input is 0 for N·L ≤ 0, the hit distance on a hit, and ≥ `NRD_FP16_MAX` on a miss.
- Forge's lamp rays use `ray_blocked`, the first-hit kind, so they would need to change.

**Several lights: there is no separate multi-light SIGMA.** The README offers two routes
[local]:
1. **A SIGMA per light,** with `maxStabilizedFrameNum = 0` so memory does not grow with the
   lights.
   - This fits a global light index. Forge's three slots change lamps from pixel to pixel, so a
     per-slot SIGMA would blur across lamp boundaries [inferred].
   - About 0.16 ms an instance at 1600×900 [inferred: 0.40 ms at 1440p on an RTX 4080, scaled by
     pixels]; three slots come to about 0.5 ms.
2. **`SIGMA_SHADOW_TRANSLUCENCY` as a single pass.**
   - Input: translucency = Σ(Lᵢ·Sᵢ)/Σ(Lᵢ) per RGB channel, and a penumbra Σ(wᵢ·Pᵢ)/Σ(wᵢ) with
     wᵢ = luminance(Lᵢ) × "in penumbra".
   - Output: final = Σ(Lᵢ) × `OUT.yzw`.
   - NVIDIA's own warnings: unbiased only without spatial filtering; the denoising is "driven by
     most important light"; usable as-is only where shadows do not overlap (overlaps need an
     umbra/penumbra classification pass); do not mix 1 cd and 100 000 cd lights (the sun stays
     separate).
   - Cost: 0.45 ms at 1440p on an RTX 4080 [local, README], so about 0.18 ms at 1600×900
     [inferred]. Memory at 1080p: 50.8 MB working set, 8.4 MB of it persistent [local].

**Licence.** The NVIDIA RTX SDKs licence. The owner allowed SDK use under licence in D-065, which
also records that the licence has no express patent grant.

## 2. The ratio estimator (Heitz, Hill, McGuire, I3D 2018)

**What it is.** E. Heitz (Unity), S. Hill (Lucasfilm), M. McGuire (NVIDIA), "Combining Analytic
Direct Illumination and Stochastic Shadows"
([NVIDIA Research](https://research.nvidia.com/publication/2018-05_combining-analytic-direct-illumination-and-stochastic-shadows),
[author page](https://eheitzresearch.wordpress.com/705-2/); abstract [read]).
- Shadowed light = unshadowed light × an illumination-weighted shadow, ∫LV/∫L.
- The first term is analytic and free of noise. Only the second is traced and denoised, so the
  blur touches shadows and never textures or normal maps.

**How it is denoised [recalled].**
- The numerator ⟨LV⟩ and the denominator ⟨L⟩ are estimated with the same samples.
- Both are filtered with the same weights, and the ratio of the filtered sums multiplies the exact
  analytic term. The ratio itself is not what gets filtered.
- An equivalent form needs one image: filter r = ⟨LV⟩/⟨L⟩ with each tap weighted by ⟨L⟩
  [inferred, algebra].
- Why it suits Forge: the fourth, stochastically picked lamp (counted as L/p) enters both terms,
  so its selection noise largely cancels. Where pixels hold different lamp sets, the ratio of
  sums stays consistent [inferred].

**Colour.** Lamps differ in kelvin, so take the ratio per RGB channel, as NRD's recipe does. On
irradiance alone, the albedo cancels [inferred].

**Highlights.** Applying the diffuse-weighted ratio to highlights is an approximation; small
lamps' highlights are tiny [inferred].

**Precedent.**
- NRD documents the same identity for its multi-light mode [local].
- Unity HDRP denoises its ray-traced point, spot and area shadows with a spatio-temporal filter,
  with an optional "distance based denoising". Its docs do not say whether it uses the ratio
  ([HDRP manual](https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/Ray-Traced-Shadows.html)
  [read]).

**Patents.** Three Google Patents searches (by technique, by Unity/Heitz/McGuire, by "unshadowed
ratio") found no claim on the ratio estimator [search]. The split is an algebraic identity
published in May 2018, so later filings face it as prior art. The search was not exhaustive.

## 3. Edge-aware spatial filters and temporal accumulation on visibility

**The techniques.**
- **Cross or joint bilateral** (Eisemann & Durand 2004; Petschnigg et al. 2004) [recalled].
- **Interleaved sampling with a geometry-aware box** over the block: Keller & Heidrich,
  "Interleaved Sampling", EGWR 2001
  ([EG digital library](https://diglib.eg.org/handle/10.2312/EGWR.EGWR01.269-276) [search]).
  Wald et al., EGWR 2002, used it with a "discontinuity buffer" [recalled].
- **Edge-avoiding à-trous**: Dammertz et al., HPG 2010
  ([PDF](https://www.uni-ulm.de/fileadmin/website_uni_ulm/iui.inst.100/institut/Papers/atrousGIfilter.pdf)
  [search]). No patent found [search].
- **SVGF**: Schied et al., HPG 2017 [search].
- **Boksansky et al., Ray Tracing Gems 2019, ch. 13**: interleaved light-sample sets matched to a
  1×1–9×9 cross-bilateral kernel and 4 frames of reuse (read for #172).

For one shadow term before TAA, #172's rule holds: filter visibility in [0, 1] before shading, at
fp16, never at 8 bits.

**NVIDIA patents met [read on Google Patents].**
- **US 10,600,167 B2,** "Performing spatiotemporal filtering" (the SVGF family; Schied, Salvi,
  Kaplanyan, Lefohn, Burgess, Patney, Wyman).
  - Priority 2017-01-18, granted 2020, active (4th-year fee paid 2023), to about 2038-07-07.
  - Claim 1: a temporal filter that back-projects each sample, checks it lies on the same surface,
    and outputs colour plus *auxiliary information* passed to later filters. Variance appears only
    in the dependent claims.
- **US 11,600,036 B2,** "Spatiotemporal self-guided shadow denoising" (Kozlowski, Panteleev).
  - Priority 2020-09-04, active to about 2041-01-23; continuation US 12,482,168 B2, later filing
    US 2026/0080606 A1.
  - Claim 1: a band of acceptable neighbour values from the first and second moments of the
    accumulated samples. Claim 17: for binary visibility, the second moment derived from the first
    (variance = m(1−m)).
- **US 11,727,535 B2,** "Using intrinsic functions for shadow denoising" (Kozlowski).
  - Priority 2021-01-14, active to about 2041-01-14; continuation US 12,505,515 B2.
  - Wave sums in the tracing warp form a penumbra mask that skips the filter on uniform tiles
    (abstract read, claims not read).
- **US 10,740,954 B2,** "Shadow denoising in ray-tracing applications" (Liu; Hasselgren,
  Munkberg, Llamas, Wyman).
  - Priority 2018-03-17, active to about 2039-03-15; continuations US 11,367,240 B2 (granted
    2022), US 12,026,822 B2, US 2024/0338884 A1.
  - Claim 1 requires an anisotropic kernel. **Claims 10 and 15 do not:** they need only a
    footprint from a shape projected from the light and the point-to-occluder distance, and a
    filter dimension taken from that footprint. This corrects `shadow-denoising.md` §3.2, which
    judged SIGMA's isotropy as narrowing the resemblance.

**Bearing [inferred, not a legal opinion].** A design that keeps clear of these claims as
summarised has:
- no filter width from a hit distance;
- no moments or variance band;
- no temporal stage whose by-products steer a later filter;
- no wave-intrinsic mask.

TAA's own accumulation, last in the frame, already exists. US patents run 20 years from filing
(plus any adjustment), so filings on the 2001–2004 techniques have lapsed; for example Keller's
mental images QMC family, US 7,432,935 B2, shows "Expired – Lifetime" (2022) [read].

## 4. Blue noise and spatiotemporal blue noise

**STBN.** Wolfe, Morrical, Akenine-Möller, Ramamoorthi, "Spatiotemporal Blue Noise Masks", EGSR
2022 ([PDF](https://cseweb.ucsd.edu/~ravir/stbn.pdf) [search]). Masks blue in space and in time
converge faster under exponential temporal filters.

**NVIDIA patents [read: bibliography and description; claims not reached].**
- **US 12,283,028 B2:** scalar masks from a modified void-and-cluster energy (A. Wolfe; priority
  2021-06-02; active to about 2043-06-12; family US 2025/0328996 A1, DE and CN filings).
- **US 11,727,621 B2:** vector masks from a modified BNDS energy (priority 2021-06-02; active to
  about 2041-12-16; continuations US 12,154,213 B2, US 2026/0148488 A1).

**The SDK cannot ship.** NVIDIA's STBN repository (now `NVIDIA-RTX/STBN`) is under the "NVIDIA
Source Code License … Non-Commercial": research or evaluation only [read, GitHub's licence API].

**Alternatives.**
- Ulichney's void-and-cluster: US 5,535,020 expired in 2013 [read].
- Christoph Peters' void-and-cluster textures are CC0
  ([momentsingraphics.de](https://momentsingraphics.de/BlueNoise.html) [read]), animated by a
  per-frame offset.
- Forge's own Hilbert R2 and interleaved gradient noise.
- NRD wants blue noise animated over 4–8 frames for SIGMA [local].

**Bearing [inferred].** For a light's disc, a deterministic stratified table interleaved over a
pixel block (§7) beats any noise mask: the noise only decides the order, and the filter needs every
index inside its window.

## 5. Alternatives that avoid denoising

- **Cube shadow maps for a few lamps** (Williams 1978; PCF, Reeves 1987).
  - No live patents on the basics. Soft edges would use PCSS: NVIDIA's US 8,462,156 B1
    (Fernando, filed 2005-12-22, fees paid through year 12) lapses about 2026-11-14 [read].
  - Against it: Forge has no raster shadow path, 6 faces a lamp, resolution aliasing and acne,
    and the owner sees shimmer. Not now.
- **More rays only for the strongest lamp.**
  - D-065 measured three lamps × three rays a pixel at +0.32 ms (`shading/standard`
    0.16 → 0.48 ms), so 8 rays for one lamp cost about +0.6 ms [inferred].
  - A pixel still reaches only 1/64 steps on its own, and the dither stays.
  - A fixed rule (the strongest lamp gets N rays) is outside NVIDIA's adaptive-ray filing as its
    summary reads: US 2024/0303907 A1 / US 12,682,550 B2 (Story, priority 2019-03-18, to about
    2040-03-18), ray counts from the neighbourhood's visibility and hit-distance variation
    [read, abstract].
- **A PCSS-like width from the ray's blocker distance.**
  - Exactly US 10,740,954's claims 10 and 15, live to about 2039.
  - Also not exact where penumbrae overlap (`shadow-denoising.md` §3.6). Avoid it, unless through
    NRD.

## 6. Patents at a glance

| Technique | Patent | Holder | Status, expiry | Alternative |
|---|---|---|---|---|
| Filter width from the occluder's distance (SIGMA-like, PCSS with rays) | US 10,740,954 B2; US 11,367,240 B2, US 12,026,822 B2, US 2024/0338884 A1 | NVIDIA | active, ~2039-03-15 | a fixed-size filter; NRD under licence |
| A temporal stage's auxiliary output steering later filters (SVGF) | US 10,600,167 B2 | NVIDIA | active, ~2038-07-07 | spatial first, TAA last |
| A moments band, or binary variance, for shadows | US 11,600,036 B2, US 12,482,168 B2 | NVIDIA | active, ~2041-01-23 | no variance input |
| A wave-intrinsic penumbra mask | US 11,727,535 B2, US 12,505,515 B2 | NVIDIA | active, ~2041-01-14 | none needed (no tile skip), or a separate classify pass |
| Adaptive shadow rays | US 2024/0303907 A1 / US 12,682,550 B2 | NVIDIA | active, ~2040-03-18 | a fixed ray budget |
| STBN masks | US 12,283,028 B2; US 11,727,621 B2, US 12,154,213 B2 | NVIDIA | active, ~2043 / ~2041 | an interleaved table; CC0 void-and-cluster |
| PCSS (shadow maps) | US 8,462,156 B1 | NVIDIA | active, lapses ~2026-11-14 | — |
| Void-and-cluster dither | US 5,535,020 | DEC/HP | expired 2013 | — |
| The ratio estimator; interleaved sampling; cross-bilateral; à-trous | none found | — | — | — |
| Forward+ tiled light culling (met in passing) | US 10,803,655 B2, US 12,136,165 B2 | AMD | active, ~2036-01-11 | not this task: check before Olsson's clusters for the streets' lamps (`interior-lighting.md`) |

The expiry dates are Google's estimates, not legal conclusions. Nothing here is a legal opinion.

## 7. Recommendation for Forge

> **Built on 2026-10-18** (D-065, "The penumbra filter"), with the rays before the resolve as
> proposed and two changes, each measured: the filter is a 7 × 7 tent (the 4 × 4 box twice over;
> the box left 4-pixel steps on the penumbrae that TAA's clamp kept), and the strongest lamp's
> second ray goes to the run paired with the frame's, not frame + 4.

**Build the interleaved ratio filter.** It uses only the ratio estimator, interleaved sampling and
a geometry-aware box: no NRD, no new rays, no patent found. Keep SIGMA's translucency route (§1) as
the upgrade, behind the same images.

1. **The lamps' sample points.**
   - A constant table of 128 disc points: the first 128 of a 2D (0,2)-sequence (Sobol), mapped by
     Shirley and Chiu's concentric map. Each aligned chunk of 16 is stratified, and so are all 128.
   - Index = 16 × (`noise_frame` mod 8) + B[x mod 4, y mod 4], B a 4×4 Bayer permutation. The
     strongest lamp's second ray uses frame + 4.
   - Keep the per-lamp golden-angle turn and drop the per-pixel turn: any sliding 4×4 window then
     holds all 16 indices once.
2. **Trace before the resolve.**
   - `lamps_at`'s choice and rays move into `shadow/trace`, which already decodes position and
     normal for the sun. Keep `ACCEPT_FIRST_HIT`: no distances are needed.
   - Write `lamp_ratio` (RGBA16F): rgb = Ŝ/Û per channel, Ŝ and Û the same-sample shadowed and
     unshadowed irradiance (the picked lamp at w/p); a = luminance(Û); (1, 1, 1, 0) where Û = 0.
3. **`shadow/lamp filter`,** a compute graph pass.
   - 8×8 groups with a 2-pixel apron: 12 × 12 × 16 B ≈ 2.3 KB of groupshared memory. No wave
     intrinsics, no float atomics, a fixed summation order.
   - The window is the 4×4 box at offsets −2..+1.
   - Weights: luminance(Û_q) × plane distance × (n·n′ > 0.9) × the same D-068 region (rooms hold
     different lamps).
   - R = Σ w·Y·r / Σ w·Y, written to RGBA16F.
4. **The resolve.**
   - `lamps_at<0>` sums every lamp of the region exactly, unshadowed and without rays, for diffuse
     and highlights. That sum is multiplied by R.
   - Without the filter, keep today's path (`--no-lamp-filter`) for an A/B.
5. **TAA or DLAA** follows unchanged. Their clamp sees a smooth signal, and over the 8-frame cycle
   the history integrates all 128 points a lamp (256 for the strongest).

**Cost at 1600×900 [inferred].** The rays are unchanged, only moved. One more RGBA16F is written
and read (about 0.03 ms), and the 16 taps take about 0.05–0.1 ms. The resolve's ray-free sum over a
region's few lamps is small. About 0.1–0.2 ms in all.

**Limits [inferred].**
- Hard contact edges soften by about ±2 pixels. A/B a 3×3 interleave (72 points) if that shows.
- No penumbra-adaptive width: very wide penumbrae lean on TAA across the cycle.
- If they still crawl in motion, the next step is one of:
  - an à-trous pass at step 4 on windows that mix lit and shadowed pixels;
  - SIGMA_SHADOW_TRANSLUCENCY fed with the same ratio and a luminance-weighted penumbra from the
    local `PackPenumbra`. That needs closest-hit rays, NRD present, about 0.2–0.45 ms, and the
    owner's word given §3's patents.

**Checks** (D-065's scripts):
- **Stills:** the hall at 22:00 and a campfire scene, frames 10 apart, % of pixels changing by more
  than 2 codes under DLAA and TAA. Today they are 0.004 % and 0.65 %.
- **Motion:** the dolly against `--ssaa`.
- **ꟻLIP:** on penumbra crops, against a new `--lamp-reference` (128–256 stratified rays a lamp,
  no filter, 64 frames).
- **The gate:** the mesh path matching the fallback; `FORGE_ASYNC=0` identical; the new zone in F1
  and `PROFILE.md`.
- **Credits:** Heitz et al. 2018, Keller & Heidrich 2001, Shirley & Chiu 1997, Sobol 1967.

## Sources

- NRD v4.18.0: README, `NRD.hlsli`, `NRDSettings.h`, `SIGMA_Blur.cs.hlsl`, `SIGMA_Config.hlsli`,
  `UPDATE.md` [local]; [releases](https://github.com/NVIDIA-RTX/NRD/releases) [read].
- Heitz, Hill, McGuire 2018: the links in §2.
- [Unity HDRP ray-traced shadows](https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/Ray-Traced-Shadows.html)
  [read].
- Olejnik & Kozlowski, "Raytraced Shadows in Call of Duty: Modern Warfare", 2020
  ([Activision](https://research.activision.com/publications/2020-10/raytraced-shadows-in-call-of-duty--modern-warfare))
  [search]: it denoises several local lights a pixel at once; the slides failed to load.
- Patents (Google Patents) [read]: [US10740954B2](https://patents.google.com/patent/US10740954B2/en)
  with its claims; [US10600167B2](https://patents.google.com/patent/US10600167B2/en) with its claims;
  [US11600036B2](https://patents.google.com/patent/US11600036B2/en) with its claims;
  [US11727535B2](https://patents.google.com/patent/US11727535B2/en);
  [US20240303907A1](https://patents.google.com/patent/US20240303907A1/en);
  [US12283028B2](https://patents.google.com/patent/US12283028B2/en);
  [US11727621B2](https://patents.google.com/patent/US11727621B2/en);
  [US8462156B1](https://patents.google.com/patent/US8462156B1/en);
  [US5535020A](https://patents.google.com/patent/US5535020A/en).
