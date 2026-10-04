# Denoising the sun's ray-traced soft shadows (issue #172)

Found on Sponza (#171), at noon from the courtyard
(`physics-lab --lab models --model Sponza --time-of-day 0.5 --view=8.84,1.83,0.39,66.2,8.3`):
the sun's shadow on the curtains has a hard, stepped edge that crawls when the camera moves. The
owner chose a shadow denoiser (2026-10-04): a filter whose width follows the penumbra estimated
from the blocker distance, plus temporal accumulation. This file is step 1 of the issue; the
🟡 decision entry is step 2.

**Status (2026-10-04):** decided as D-049 (NRD's SIGMA itself, not a rewrite) and built in #172.
The measurements are in D-049. AMD's FidelityFX denoiser (§2) is ported as the fallback without
NRD (#173). As §2 expected, its reach of about 8 pixels falls short of the widest penumbrae.

Today:
- **The ray.** `sun_shadow` (`shaders/meshlet.slang`, issue #54, D-029) traces one shadow ray per
  sun-facing pixel inside the resolve, as an inline ray query that stops at the first hit
  (`RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH`). It returns 0 or 1, and the resolve multiplies it
  into the sun's light, then by `cloud_light`.
- **Where it aims.** `sun_disc_point` picks one of 8 points of a Vogel disc on the sun's disc
  (0.27° in radius), one a frame (`noise_frame`, which repeats with TAA's jitter: 16 frames in the
  city and the labs), turned by an angle of the pixel's own from `spatio_temporal_noise`.
- **No denoiser:** TAA averages the frames into the penumbra.
- **Other rays.**
  - The water's shadow ray (`trace_requested`) aims through the same `sun_disc_point`. It
    subtracts the shadowed share of a sun term the water pass stored.
  - The probes trace the disc's centre.
- **Cost:** the soft shadows cost 0.02 ms at 1440p (D-029).
- **TAA** (`shaders/taa.slang`):
  - It reads a 3×3 neighbourhood and compresses colour with Karis's `c / (1 + luma)` before
    averaging.
  - It clips the history in YCoCg to the mean ± 1.25 standard deviations.
  - It blends 0.1 of the current frame (0.04–0.1 by the sample's weight), more as the motion
    grows (|motion| / 48 px).
- **Guide images:** there is no normal image. The resolve decodes the visibility buffer, and the
  depth and the motion vectors are images of their own.

Sources were read with WebSearch, WebFetch and the GitHub API on 2026-10-04. Claims taken from a
search result's summary, not from the page or the code, are marked †.

## 0. Why the penumbra breaks under that exposure (reading Forge's code)

This section is arithmetic on the shaders above, not a source.

The exposure is metered for the arcade, about 8 stops under direct sun. Call the shadowed side's
pre-exposed luminance s (say 0.2); the sunlit side is then about 2⁸ × s ≈ 51. A pixel whose true
visibility is v should show s + v · 51.

**What the picture should be.** The display saturates a few units above s, so everything above
v ≈ 0.1 is white. The visible gradient of a correct penumbra lives in v ∈ [0, ~1/8], next to the
umbra.
- For a disc and a straight edge, v grows like x^1.5 from the umbra's side (v ≈ 0.6 (x/r)^1.5
  for a disc of radius r), so v = 1/8 is reached a fifth of the way across: about 3 of Sponza's
  16 pixels (an estimate).
- Resolving that ramp smoothly needs visibility to about 1/256, the step that one display-level
  change takes at ×256.
- Eight disc points give steps of 1/8. That is the one hard step the issue describes.

**What TAA makes of it.** TAA does not average v. It averages c = L / (1 + L) and expands the
result, so the per-frame samples are c(0.2) = 0.167 and c(51) = 0.981, and the history converges
to m = 0.167 + 0.814 v, shown as m / (1 − m):

| v | 1/8 | 1/4 | 1/2 | 3/4 | 7/8 | 1 |
|---|---|---|---|---|---|---|
| correct, s + 51 v | 6.6 | 13 | 26 | 39 | 45 | 51 |
| TAA's converged value | 0.37 | 0.59 | 1.35 | 3.5 | 7.3 | 51 |

- **The result is biased dark.** The ramp moves out to v ≈ 0.6–0.95, the lit rim, where the
  issue sees the step.
- **The expand is steep there.** Its slope is 1/(1 − m)², about 70 at v = 7/8. The history is an
  exponential average (α ≈ 0.1) of a sequence that cycles through 8 points. It ripples by about
  ±0.04 in m, which at v = 7/8 swings the shown value between about 5 and 11 from frame to frame.
  That is the crawl, even in a still frame.
- **The clip does not hold the history.** Its box is built from 3×3 binary samples, so it spans
  nearly [s, lit] and holds nothing. In motion α rises and the per-pixel turn shows as dither.

Two things follow. The fix is to denoise the visibility in [0, 1], before shading, with enough
effective samples, and to hand TAA a smooth signal. And a correct penumbra at 8 stops over is
narrower than today's smear: most of the geometric penumbra is white.

## 1. NVIDIA NRD's SIGMA

**NVIDIA, "NRD — NVIDIA Real-time Denoisers", v4.18, GitHub, 2026**
([repository](https://github.com/NVIDIA-RTX/NRD),
[README](https://raw.githubusercontent.com/NVIDIA-RTX/NRD/master/README.md); read with the
pass list in `Source/Denoisers/Sigma_Shadow.hpp`, the front end in `Shaders/NRD.hlsli`, the switches
in `Shaders/SIGMA_Config.hlsli`, the settings in `Include/NRDSettings.h`).

**What it is.** A shadow-only denoiser for one light at a time, either an infinite light (the
sun, the Moon) or a local one (omni, spot). `SIGMA_SHADOW_TRANSLUCENCY` adds coloured shadows
through translucent occluders.

**Inputs.**
- `IN_PENUMBRA`, from `SIGMA_FrontEnd_PackPenumbra(distanceToOccluder, tanOfLightAngularRadius)`:
  the penumbra's half-size in world units, hit distance × tan(angular radius) × 0.5. A miss is
  stored as `NRD_FP16_MAX` (lit). A pixel facing away gets 0.
- `IN_VIEWZ`, `IN_NORMAL_ROUGHNESS` and `IN_MV`.
- `IN_TRANSLUCENCY` for the translucency variant: the RGB transmittance crossed, and whether the
  ray missed.
- **The rays.** They go from the surface to the light. They need the *closest* hit: the README
  rules out `ACCEPT_FIRST_HIT_AND_END_SEARCH`, because a random distant occluder gives a wrong,
  long distance. Forge's shadow ray uses that flag today.
- **The output is stored as a square root.** `SIGMA_BackEnd_UnpackShadow(x)` is `x * x`, which
  keeps precision at the dark end.

**Settings.**
- `planeDistanceSensitivity` (0.02).
- `maxStabilizedFrameNum` (5 by default, at most 7; NRD says to count the accumulation in time,
  0.084 s by default).
- The sun's direction, and a checkerboard mode for half the rays.

**Passes** (`Sigma_Shadow.hpp`):
1. **Classify tiles** (16×16, from view Z and the penumbra).
2. **Smooth tiles.**
3. **Copy**, the history.
4. **Blur.** A dense 5×5 estimate of the penumbra from the neighbours' hit distances, which
   prefers the smaller ones. Lit pixels have no hit distance, so they take their neighbours'.
   Then 12 Poisson taps over a radius set by the penumbra in pixels (at most 32 px), turned per
   pixel and frame, weighted by a Gaussian and by the distance to the centre's tangent plane.
   Tiles that are fully lit or fully in shadow skip it.
5. **Post-blur:** the same, with another rotation.
6. **Temporal stabilization:** a Catmull-Rom reprojection, a 5×5 variance clamp, and at most 7
   accumulated frames.

**Cost.**
- Time: 0.40 ms (`SIGMA_SHADOW`) and 0.45 ms (translucency) at 1440p on an RTX 4080, with
  default settings.
- Memory: 56 MB (15 MB persistent, the rest aliasable) and 90 MB.

**Cross-vendor.**
- NRD is HLSL compute, for D3D12 and Vulkan through NVIDIA's NRI layer or the integrator's own.
- SIGMA's blur, classification and stabilization use no wave intrinsics: small groupshared
  preloads, and groupshared `InterlockedMin`/`Max` on integers in the tile pass (read in the
  shaders).
- The README advises blue noise with "a limited number of animated frames (4-8)", or a static
  pattern.

**Licence.**
- NRD is under the **NVIDIA RTX SDKs License** ([LICENSE.txt](https://raw.githubusercontent.com/NVIDIA-RTX/NRD/master/LICENSE.txt)),
  not MIT. `lighting-gi.md` still says MIT; the decision entry should correct that.
- It costs nothing and has no royalty. Applications may ship it as object code. Modified sample
  source must carry an NVIDIA notice.
- The SDK may not be made subject to an open-source licence. The shader headers say any
  reproduction without a licence agreement is prohibited.

*Bearing:*
- SIGMA is the design to follow:
  - inputs Forge can produce (hit distance, view Z, normal, motion);
  - the order: blur first, then a temporal pass whose clamp works on the blurred signal;
  - the penumbra in lit pixels gathered from blocked neighbours;
  - a square-root store.
- Its code cannot be translated into Forge's MIT/Apache shaders under its licence. A Forge
  version must come from the published techniques below, not from NRD's HLSL.
- Using NRD as a library is possible, but costs a lot: HLSL outside `slangc`, NRI or a mapping of
  NRD's dispatch list onto graph passes with `Custom` accesses, the licence's terms, and a
  download.

## 2. AMD FidelityFX Shadow Denoiser

**AMD, "FidelityFX Denoiser: Shadows", GPUOpen, 2021**
([repository, MIT](https://github.com/GPUOpen-Effects/FidelityFX-Denoiser),
[pass-by-pass document](https://raw.githubusercontent.com/GPUOpen-Effects/FidelityFX-Denoiser/master/docs/FFX_Denoiser_Shadows_Technology.pdf),
[FidelityFX SDK manual](https://gpuopen.com/manuals/fidelityfx_sdk/techniques/denoiser/),
[product page](https://gpuopen.com/fidelityfx-denoiser/)). The product page lists version 1.1.4
(May 2025) in FidelityFX SDK 1, for DirectX 12 and Vulkan; the shaders were read in
`ffx-shadows-dnsr/`.

**What it is.** A spatio-temporal denoiser for ray-traced *soft* shadows towards one light,
"designed with one ray per pixel in mind": the rays are jittered over the light, at most one per
pixel.

**Inputs.**
- The hit mask, packed one bit per pixel into a 32-bit word per 8×4 tile.
- The depth, the motion vectors and the normals.
- The previous depth, moments and history.
- **No hit distance:** the filter knows nothing about the penumbra's size.

**Passes.**
1. **Prepare** packs the mask with `WaveActiveBitOr`, which assumes an 8×4 tile lies in one
   wave. A trace pass that writes the words itself can skip it.
2. **Tile classification.**
   - The moments of a 17×17 neighbourhood, made separable and read as 18 scalar loads from the
     bitmask.
   - A disocclusion mask with a depth threshold that grows at grazing angles.
   - The reprojection, keeping per pixel the mean, the variance and the sample count.
   - The variance is boosted under 16 samples, the history clamped to mean ± 0.5σ, and the
     history weight set from the count.
   - Tiles with no variance are skipped (`WaveActiveAllTrue`, with a groupshared fallback when
     the wave is not the group).
3. **Three filter passes:** an edge-avoiding à-trous 3×3 (weights 1, 2/3, 1/6) with growing
   steps. The edges stop on depth, normal, and shadow similarity scaled by the filtered variance.
   The variance is refiltered after each pass. Each pass uses 4 KB of groupshared memory (four
   16×16 arrays) and fp16.

**Notes.** AMD found the filter passes faster in wave32 and the classification in wave64; that is
performance only. No published cost was found.

*Bearing:*
- The FFX denoiser is portable and its licence is clean: MIT, ported into Slang with its notice
  in the file and in `CREDITS.md`.
- Forge's rules need two changes. The prepare pass's wave-wide OR must go: the trace writes plain
  visibility instead, since one sun does not need the bandwidth trick. And `shaderFloat16`, which
  Forge's device already enables.
- It is weak where #172 needs strength. No distance input means the blur width follows the noise,
  not the penumbra. The 0.5σ clamp of a 1-spp binary signal leaves either ghosting or noise in
  motion. Three 3×3 à-trous passes reach about 15 px, short of the wide penumbrae of a distant
  occluder.

## 3. Other approaches

### 3.1 Adaptive sampling and penumbra-aware filtering (Ray Tracing Gems, ch. 13)

**J. Boksansky, M. Wimmer, J. Bittner, "Ray Traced Shadows: Maintaining Real-Time Frame Rates",
Ray Tracing Gems, 2019, ch. 13, pp. 159–182** ([author's PDF](https://boksajak.github.io/files/RTG1_RayTracedShadows.pdf),
[TU Wien](https://www.cg.tuwien.ac.at/research/publications/2019/BOKSANSKY-2019-RTS/); open access,
CC BY-NC-ND).

**Separate visibility from shading.** The light's shading uses its centroid, and the visibility
goes in a buffer per light (their eq. 5).

**Temporal reuse.**
- Four previous frames are reprojected.
- Their depth test is relative, with a threshold ε = c₁ + c₂|n_z| (c₁ = 0.003, c₂ = 0.017).

**Where the penumbrae are.**
- The variation Δv is max − min of the four frames' visibility.
- It is spread by a 5×5 maximum and a 13×13 tent, and averaged with the last four frames.

**Adaptive rays.**
- 0 to s_max rays a pixel (5, or 8 for high quality).
- One more when Δv is over a threshold, one fewer after four stable frames.
- A 4×4 pattern of 8×8 blocks forces one ray per pixel once every four frames.

**The filter.**
- A temporal box over the four frames.
- Then a cross-bilateral Gaussian of 1×1 to 9×9, its size from the variation (largest at
  Δv = 0.4), interpolated between the precomputed kernels so it does not pop.
- Taps are rejected by the depth test and by n·n′ < 0.9.

**Sample sets.**
- s_max × 4 × 3 × 3 Poisson points on the light, optimised so that each frame's quarter and each
  3×3 pixel's ninth are well spread: 36 points for one ray a pixel, 288 for eight.

**Cost** (RTX 2080 Ti, 1080p, one light, rays included):
- 2.7–4.7 ms soft against 1.3–1.6 ms hard.
- 4 rays a pixel: 3.6 ms; their 0–5 adaptive rays: 2.7 ms.

*Bearing:*
- The sample-set design is the direct cure for the "eight stripes": spread many distinct disc
  points over a pixel block and the frames, matched to the filter's footprint, so a small filter
  over a few frames integrates hundreds of points.
- The kernel comes from the variation, so no blocker distance is needed. But it stops at 9×9,
  short of Sponza's 16 px.
- The 2019 costs predate the 5070 Ti by three generations; Forge's own rays cost 0.02 ms.

### 3.2 Unreal Engine 4's shadow denoiser and NVIDIA's patent

**E. Liu, I. Llamas, J. Cañada, P. Kelly, "Cinematic Rendering in UE4 with Real-Time Ray Tracing
and Denoising", Ray Tracing Gems, 2019, ch. 19**
([Springer PDF](https://link.springer.com/content/pdf/10.1007/978-1-4842-4427-2_19.pdf); the
host refused the fetch, so its content here is from search summaries).
- One ray a pixel for each area light†.
- The denoiser takes the light's size, shape, direction and distance from the receiver, and the
  shadow rays' hit distances, and derives a spatial footprint for each pixel, anisotropic, with
  directions that vary per pixel†.

**NVIDIA, US 10,740,954 B2, "Shadow denoising in ray-tracing applications"**
([Google Patents](https://patents.google.com/patent/US10740954B2/en)).
- Filed with priority 2018-03-17, granted 2020-08-11, active, estimated expiry 2039-03-15.
- Inventors S. Liu, J. Hasselgren, J. Munkberg, I. Llamas, C. Wyman.
- Its first claim, as Google's page summarises it: a footprint computed from the distance between
  the shaded point and the occluder, projected into image space along the view, and turned into an
  *anisotropic* filter with Gaussian weights along its directions, applied to the shadow data.
- A related grant is US 11,727,535 B2 ([Google Patents](https://patents.google.com/patent/US11727535)),
  on intrinsic functions for shadow denoising†.

*Bearing:*
- The owner's sketch, a width from the blocker distance, is close to what this patent claims.
  This is not a legal reading. The decision entry should put it to the owner before Forge builds
  a distance-driven filter.
- Variation-driven filters (§2, §3.1) do not use the occluder's distance at all.
- SIGMA's screen-space mode is isotropic: its `SIGMA_USE_SCREEN_SPACE_SAMPLING` note says it
  does not elongate the shadow. That narrows the resemblance to the claim; it does not settle it.

### 3.3 The ratio estimator: denoise the shadow, not the light

**E. Heitz, S. Hill, M. McGuire, "Combining Analytic Direct Illumination and Stochastic Shadows",
I3D 2018** ([NVIDIA Research](https://research.nvidia.com/publication/2018-05_combining-analytic-direct-illumination-and-stochastic-shadows),
[ACM](https://dl.acm.org/doi/10.1145/3190834.3190852)).
- Shadowed illumination splits exactly into unshadowed illumination times an
  illumination-weighted shadow (the abstract).
- The first is computed analytically, free of noise. Only the second is traced and denoised, so
  the noise stays in the shadows†.

*Bearing:*
- This is the principle of §4. For the sun (one radiance over 0.27°, and BRDFs that barely change
  across it), the illumination-weighted shadow is the plain visibility. So Forge's denoised
  visibility times its existing sun term *is* this estimator.
- The water already works this way: it stores its sun term and subtracts the shadowed share. But
  a subtraction in fp16 cancels badly when the sun is 2⁸ times the rest. A split by
  multiplication, or by adding the shadowed sun term, keeps the dark side exact.

### 3.4 SEED's PICA PICA, SVGF, and AMD's Hybrid Shadows

**C. Barré-Brisebois, H. Halén, G. Wihlidal, A. Lauritzen, J. Bekkers, T. Stachowiak,
J. Andersson, "Hybrid Rendering for Real-Time Ray Tracing", Ray Tracing Gems, 2019, ch. 25**
([Springer PDF](https://link.springer.com/content/pdf/10.1007/978-1-4842-4427-2_25.pdf);
[GDC 2018 slides](https://media.contentapi.ea.com/content/dam/ea/seed/presentations/gdc2018-seed-shiny-pixels-and-beyond-real-time-raytracing-at-seed.pdf),
which did not decode as text). The rays sample a cone towards the light, and an SVGF-based filter
cleans the result†.

**C. Schied et al., "Spatiotemporal Variance-Guided Filtering", HPG 2017**
([NVIDIA Research](https://research.nvidia.com/labs/rtr/publication/schied2017spatiotemporal),
[PDF](https://cwyman.org/papers/hpg17_svgf.pdf)). SVGF accumulates over time, estimates the
variance in space and time, and drives an edge-stopping à-trous wavelet by it. This is the
ancestor of FFX's design. Its published cost, about 10 ms at 1080p, is for full path-traced GI
in 2017, not for one shadow†.

**AMD, "FidelityFX Hybrid Shadows" sample**
([manual](https://gpuopen.com/manuals/fidelityfx_sdk/samples/hybrid-shadows/)).
- A shadow map's blocker search estimates the penumbra, and a classifier sends rays only to the
  tiles that need them.
- The sun's size is a "Sun Solid Angle" parameter, and the FFX shadow denoiser cleans the one ray
  a pixel.

*Bearing:* Forge has no shadow map. Its blocker search is the ray's own hit distance.

### 3.5 Shipped games: Call of Duty and Cyberpunk 2077

**M. Olejnik, P. Kozlowski, "Raytraced Shadows in Call of Duty: Modern Warfare", 2020**
([Activision Research](https://research.activision.com/publications/2020-10/raytraced-shadows-in-call-of-duty--modern-warfare);
[slides](https://www.activision.com/cdn/research/Raytraced_Shadows_in_Call_of_Duty_Modern_Warfare.pdf),
whose fetch was reset twice).
- The talk covers integrating ray tracing into a Forward+ engine with almost no content changes,
  and cleaning up the noise of several local area lights.
- It denoises several lights per pixel at once†, measured on an RTX 2070 at 1440p†.
- Olejnik presents variable-rate ray tracing in Modern Warfare 4 at SIGGRAPH 2026's Advances
  course ([slides](https://advances.realtimerendering.com/s2026/content/SIGGRAPH2026%20-%20Micha%C5%82%20Olejnik%20-%20Variable%20Rate%20Ray%20Tracing%20in%20COD%20MW4.pdf);
  not read here).

**Cyberpunk 2077** traced the sun's shadows at launch and added local lights' shadows in patch 1.5†
([Wccftech](https://wccftech.com/cyberpunk-2077-gets-ray-traced-local-light-shadows-on-pc-thanks-to-partnership-with-nvidia/)).
Its ray-traced passes were denoised by NRD† ([Tom's Hardware](https://www.tomshardware.com/news/cyberpunk-2077-adding-new-nvidia-denoiser)).

*Bearing:* the shipped titles denoise each light's 1-spp visibility with a dedicated denoiser.
None of them leaves it to TAA.

### 3.6 Filtering a hard shadow by the penumbra (PCSS-like)

**R. Fernando, "Percentage-Closer Soft Shadows", SIGGRAPH 2005 sketch**
([NVIDIA PDF](https://developer.download.nvidia.com/shaderlibrary/docs/shadow_PCSS.pdf)).
- PCSS searches the blockers, estimates the penumbra as w = (d_receiver − d_blocker) ·
  w_light / d_blocker, and filters by that width.
- For a directional disc, the width across the light is 2 · d · tan θ, d being the distance to the
  blocker: about 5 cm for Sponza's ledge 5.5 m away.

**C. Soler, F. Sillion, "Fast Calculation of Soft Shadow Textures Using Convolution", SIGGRAPH
1998** ([INRIA HAL](https://inria.hal.science/inria-00510082); the host refused the fetch).
When the light, the occluder and the receiver lie in parallel planes, the soft shadow is the hard
shadow convolved with the light's projected shape†.

**For Forge.** Trace the disc's centre, one deterministic ray with its hit distance, then filter
the binary hard shadow with a disc kernel the size of the projected penumbra.
- **No noise source:** nothing to accumulate. A still frame is exactly stable, and in motion only
  the hard edge's aliasing remains, which TAA already handles.
- **Sponza is the good case.** The ledge is a straight edge at one distance, near the parallel-plane
  case, so the result is close to exact there.
- **It fails** where occluders overlap within the penumbra, where light comes through gaps smaller
  than the penumbra, and on blocker distances that vary within the kernel.
- **Lit pixels again have no distance** and gather it from their neighbours.
- **A wide kernel is costly.** Up to 32 px, it needs many taps, a mip chain of the hard shadow, or
  sparse rotated taps, which bring noise back.

*Bearing:* a cheap and very stable baseline, exact for many of the sun's simple cases, and the
reference to compare a denoiser against on Sponza. It is also the patent's territory, a footprint
from the occluder's distance.

### 3.7 More rays with low-discrepancy points

**More rays in the resolve.** Forge's soft-shadow ray costs 0.02 ms at 1440p (D-029, coherent sun
rays), so 8 rays a pixel cost about 0.16 ms and 32 about 0.6 ms (an estimate: rays in penumbrae
are less coherent).
- Visibility is still quantised to 1 / (rays × distinct points).
- Resolving 1/256 through TAA needs at least 256 distinct points a pixel in TAA's window. TAA's
  history is an exponential average, not a box, so even 16 rays × the 16-frame cycle leaves
  ripple.

**A. Wolfe, N. Morrical, T. Akenine-Möller, R. Ramamoorthi, "Spatiotemporal Blue Noise Masks",
EGSR 2022** ([Eurographics](https://diglib.eg.org/handle/10.2312/sr20221161),
[PDF](https://cseweb.ucsd.edu/~ravir/stbn.pdf),
[masks and code](https://github.com/NVIDIAGameWorks/SpatiotemporalBlueNoiseSDK)). Masks that are
blue in both space and time converge faster and stay steadier under temporal filters†. Forge
already uses a Hilbert-curve R2 (`spatio_temporal_noise`) and interleaved gradient noise for GTAO
(#48).

*Bearing:* more rays alone is a stopgap. Its useful part is the point sets: distinct disc points
across a pixel block and the frames, so that any spatial filter integrates more of the disc.

### 3.8 ReSTIR

**B. Bitterli, C. Wyman, M. Pharr, P. Shirley, A. Lefohn, W. Jarosz, "Spatiotemporal reservoir
resampling for real-time ray tracing with dynamic direct lighting", SIGGRAPH 2020**
([PDF](https://research.nvidia.com/sites/default/files/pubs/2020-07_Spatiotemporal-reservoir-resampling/ReSTIR.pdf)).
ReSTIR resamples candidate lights over space and time to choose among thousands of lights.

*Bearing:*
- With one small sun there is nothing to choose: the noise is the binary visibility.
- Reusing visibility across pixels without retracing is biased. That is a denoiser's job, done
  with the bias controlled.
- ReSTIR belongs to D-008's local-light tier, not to #172.

## 4. What keeps a penumbra stable when the lit side is 8 stops over

1. **Denoise visibility, not radiance** (§3.3, §3.1's eq. 5, SIGMA).
   - Filter v ∈ [0, 1], linear, before shading. The resolve multiplies it into the sun term,
     and TAA receives a smooth radiance its clip can hold.
   - Denoising the exposed radiance instead would mix 8 stops of contrast into every weight,
     and the Karis compression of §0 would bias it.
2. **Precision at the dark end.** Store the visibility as R16F, or as the square root SIGMA uses,
   never as linear R8. At ×256, one step of 1/255 is a whole shadow-side level.
3. **Enough distinct samples.** The visible ramp is v ∈ [0, ~1/8] and wants a resolution of about
   1/256.
   - A blur over a 16-px penumbra (radius 8, ~200 pixels), times 8–16 frames, gives thousands of
     samples.
   - The samples must be *distinct, stratified* disc points (§3.1's sets, §3.7), not the same 8
     points turned. For a straight edge, stratified points converge almost as 1/N, independent
     ones as 1/√N.
4. **The filter width from the blocker distance.** The radius in pixels is the penumbra
   (hit distance × tan θ) over the size of a pixel at that depth, foreshortened by the surface's
   angle (§1, §3.2, §3.6).
   - A contact stays sharp because its blocker is near, and a far blocker's shadow gets the wide
     kernel it needs.
   - A variance-only width blurs contacts in motion or under-filters wide penumbrae (§2). See
     §3.2 for the patent question.
5. **Accumulate over time, and clamp the filtered signal.**
   - Clamping 1-spp binary samples to a 3×3 box does nothing: the box spans [0, 1]. A clamp to
     mean ± kσ of the *blurred* neighbourhood (SIGMA's order: blur, then stabilize) rejects
     ghosts without bringing the noise back.
   - A history of at least one full cycle of disc points, or a box over the cycle as in §3.1,
     removes the cycle's ripple exactly in a still frame.
6. **Hand TAA a converged signal.** The same jitter phase then gives the same pixels from one
   cycle to the next. TAA's Karis bias only acts on geometric edges, where it belongs.
7. **A short or static noise pattern.** NRD advises 4–8 animated frames. Forge's disc points
   already repeat with the jitter.

## 5. Forge's constraints

- **Cross-vendor** (#67):
  - Nothing here needs a wave intrinsic. SIGMA's shaders use none in the blur, and FFX's prepare
    pass is the one place to replace.
  - Groupshared: an 8×8 group with a 2-pixel apron for a dense 5×5 estimate holds
    12 × 12 × (visibility, penumbra, depth) × 4 B ≈ 1.7 KB.
  - The wide blur reads its sparse taps from the image. A 32-px apron in groupshared would need
    72 × 72 × 12 B ≈ 62 KB, over the 32 KB limit.
  - Tile classification uses groupshared integer min and max, which do not depend on order.
- **Render graph** (D-020):
  - Every step is a compute pass under `shadow/…`. Each declares the visibility buffer, depth,
    motion and guide images it reads, and writes R16F/RG16F transients.
  - The history is a `GraphImage`, like TAA's, so the F1 overlay and `docs/PROFILE.md` get the
    zones.
  - The passes need this frame's geometry, so they stay on the graphics queue, or go to async
    compute after the visibility buffer if the graph allows.
- **The mesh path and the fallback at 0 px apart.** The trace pass reads the visibility buffer and
  depth, which both paths already produce identically (with the software rasteriser's pixels).
  Every pass after it is a function of those images.
- **Determinism.**
  - Compute passes, not fullscreen draws: #161 found a fullscreen draw on the serial frame giving
    one of two results.
  - Sums in a fixed order, no float atomics, and the history reset with TAA's reset.
  - The rotation of the taps and the disc points repeat with the jitter cycle, so captures repeat
    to the bit and `FORGE_ASYNC=0` gives the same frame.
- **TAA and DLSS.** The denoiser runs at render resolution before the resolve, so it is the same
  under TAA, DLAA and DLSS.
- **Hard shadows** (the ballad, `sun_angular_radius` 0) must pass through untouched: the penumbra
  is 0, so the tiles skip, giving the same pixels as today.
- **Credits** (`CREDITS.md`):
  - An in-house implementation credits the techniques: SVGF, Boksansky et al., PCSS, Heitz et al.,
    and SIGMA's design as NRD's documentation describes it.
  - Code ported from FFX carries AMD's MIT notice.
  - NRD's source cannot be ported under its licence.

## 6. Comparison

The 1440p costs are for an RTX 5070 Ti. Only SIGMA's figure is measured, and on an RTX 4080, a GPU
of the same class (this comparison is an estimate). The rest are estimates to check with the F1
overlay.

| Approach | Wide penumbrae | Narrow, contact | Still frame | Motion | Cost, 1440p (1080p ≈ 0.56×) | Complexity | Licence, IP |
|---|---|---|---|---|---|---|---|
| Today: 1 ray, 8 points, TAA | stripes | right | ripples at the lit rim (§0) | crawls, dithers | 0.02 ms | none | own |
| More rays, stratified (16/pixel) | 1/256 only with many frames | right | still ripples through TAA's average | better, not stable | ~0.3 ms | low | own |
| Hard ray + penumbra filter (§3.6) | approximate; no overlaps | right | exact, no noise | stable; hard-edge aliasing only | ~0.1–0.3 ms | low–medium | own; footprint from distance (§3.2) |
| FFX shadow denoiser, ported | under-filters past ~15 px | good | good after ~16 frames | variance clamp: ghost or noise | not published; ~0.3–0.5 ms est. | medium | MIT |
| Boksansky adaptive (§3.1) | up to 9×9 | good | good (box over 4 frames) | more rays in motion | rays + ~0.2 ms | medium–high | technique |
| NRD SIGMA as a library | good | good (dense estimate) | good | good, ≤ 7 frames | 0.40 ms on a 4080 | high (HLSL, NRI, graph mapping) | NVIDIA RTX SDKs License; download |
| **SIGMA-like, in Slang (§7)** | good (to 32 px) | good | good | good | ~0.35–0.6 ms with the trace pass | medium–high | own; patent question (§3.2) |
| ReSTIR | — | — | — | — | — | — | not for one sun |

## 7. Recommendation

**Build first: a sun-shadow denoiser of Forge's own, in Slang, with SIGMA's structure, written
from the published techniques** (§1's documented interface, §3.1, §3.3, §3.6, §4). Behind it, keep
the hard-ray penumbra filter (§3.6) as a debug reference, and the FFX port as the fallback should
the owner rule out a distance-driven width (§3.2).

**Step 0: trace before the resolve** (`shadow/trace`, a compute pass at render resolution).
- **Inputs.** It reads the visibility buffer and depth, and decodes what the ray needs: the
  position, the interpolated normal and `shadow_bias` for the instance.
- **The ray.**
  - It traces with today's origin and bias, but takes the *closest* hit. Measure the cost against
    `ACCEPT_FIRST_HIT`, which SIGMA's README rules out for distances.
  - The disc point becomes an index into a larger stratified set: about 128 points spread over a
    4×4 pixel block and 8 frames (§3.1's design), or an STBN mask (§3.7). The cycle still repeats
    with the jitter.
- **Outputs.**
  - RG16F: the visibility (0 or 1) and the penumbra radius in metres, hit distance × tan θ, or
    the FP16 maximum on a miss.
  - The view-space normal (RG16 octahedral) and the view Z as guides.
- **The resolve keeps its shading.** `s.shadow` becomes a load of the denoised visibility, times
  the facing test `dot(s.n, sun) > 0` and `cloud_light`.
- **The alternative.** The resolve writes its unshadowed sun term to an image, and a composite
  *adds* V × sun (§3.3). This saves the second decode but changes every material class. Never
  subtract in fp16. Choose it only if the decode pass costs over ~0.1 ms.

**Steps 1–4: the denoiser.**
1. **`shadow/classify`** works on 16×16 tiles: whether a tile holds any penumbra, and its largest
   radius in pixels. Fully lit, fully shadowed and hard-shadow tiles are skipped by the next passes.
2. **`shadow/blur`.**
   - A dense 5×5 estimate of the penumbra from blocked neighbours, with lit pixels taking their
     neighbours'.
   - Then a radius in pixels, clamped to 1–32, from the penumbra over the pixel's footprint at its
     depth.
   - 12–16 Poisson taps, turned by an angle that repeats with the jitter cycle, weighted by a
     Gaussian × the plane distance × the normals' agreement.
3. **`shadow/post-blur`:** the same, at a smaller radius and another rotation.
4. **`shadow/temporal`.**
   - It reprojects through TAA's motion vectors and detects disocclusion by the plane distance.
   - A history of up to 16 frames (one jitter cycle), clamped to mean ± 1–1.5σ of the post-blur's
     5×5.
   - The output is R16F, read by the resolve.

**What the resolve side must provide.** The visibility buffer and depth (both exist), the
motion vectors (they exist, at render resolution), the sun's angular radius and direction
(`Frame`), the jitter cycle's index (`noise_frame`), and TAA's history reset.

**Expected cost at 1440p on the 5070 Ti.**
- The trace pass: 0.05–0.15 ms, the decode to measure, plus the rays at about 0.02–0.05 ms.
- The four denoiser passes: 0.3–0.45 ms, with SIGMA's 0.40 ms on an RTX 4080 as the yardstick
  (a pass that skips most tiles costs less).
- About 0.2–0.35 ms at 1080p.
- Memory: a dozen MB of R16F/RG16F images at 1440p, most of them aliased transients; the
  history is persistent.

**Later.**
- The water's shadow ray through the same denoiser (it already stores its sun term).
- Extra rays only in penumbra tiles (§3.1) if one ray a pixel converges too slowly in motion.
- RGB transmittance through translucent occluders (SIGMA's translucency) when glass casts shadows.

**Questions for the decision entry.**
1. A width from the blocker distance (the owner's sketch; §3.2's patent), or a variation-driven
   width (FFX, MIT; Boksansky)?
2. Is the narrower, correct penumbra at 8 stops over (§0) the look the owner wants? A larger sun
   disc for art's sake is a separate knob.
3. May NRD be downloaded as an A/B yardstick, never shipped?

## 8. How to verify it

- **The view:** `physics-lab --lab models --model Sponza --time-of-day 0.5
  --view=8.84,1.83,0.39,66.2,8.3`, TAA on, at 1080p and 1440p.
- **A reference:** a development option that traces 256 stratified disc points a pixel each frame
  with no denoiser, captured after 64 frames. It is the target for the penumbra's shape at that
  exposure (§0), and §3.6's filter is a second, independent check on the straight ledge.
- **Still-frame shimmer over TAA's 16-frame cycle:**
  - capture frames 600–631 (`--capture-every 1`) and crop the curtains' penumbra;
  - per pixel, the range (max − min) of the displayed signal over each 16-frame cycle: its mean
    and 99th percentile in the crop, before and after;
  - the slow change: the share of pixels that differ between frames 600 and 616 (the same phase)
    in `imgdiff`.
  - Targets: a range under 2 codes in the crop, and a slow change back to hard shadows' 0.035 %
    (D-029).
- **ꟻLIP:**
  - `imgdiff reference.png denoised.png` on the crop and the whole frame: its mean and its largest
    value, before and after. The images the change alters go through `--expect`.
  - The same against today's captures of the city's south view, the island and the ballad (whose
    hard shadows must not change).
- **Motion:** `imgdiff --then` between a slow orbit or pan and the same path with the reference
  (#65's tool), and the real-time tour for the owner.
- **Debug views:** the raw and denoised visibility, the penumbra radius in pixels, the skipped
  tiles and the history's length.
- **The gate:** the mesh path and the fallback at 0 px; `FORGE_ASYNC=0` repeating to the bit;
  validation clean; Tier 1, since it is shared rendering. The `shadow/*` zones go into
  `docs/PROFILE.md` with `--timings` against the base.
