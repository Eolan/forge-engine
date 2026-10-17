# Research — Local exposure: a bilateral grid of log luminance after the resolve, exposure fusion, and their patents

> Written 2026-10-17 for issue #275. Walking into a village house, the owner wrote: "light outside is very strong, I would expect the inside to be a bit brighter". This note is a companion to `lighting-gi.md` §5 and §10 (exposure and the display transforms, D-022), `hdr-output.md` (the transforms on HDR displays) and `interior-lighting.md` (#270: what lights the rooms). Every citation was checked on 2026-10-17 in one of three ways:
> - *read*: fetched and read that day;
> - *search-recorded*: seen only through the search engine's record of it, because the host refused the fetch, the page is drawn by script, or its PDF could not be read as text;
> - *from memory*: said so where it is used.
>
> [Verification notes](#verification-notes) lists each source by kind. The patent facts are Google Patents' records, whose status and expiry fields are its own assumptions, not legal conclusions. No freedom-to-operate search was made, and nothing here is legal advice. Forge's own facts were read on 2026-10-17 from:
> - `crates/forge-render/src/exposure.rs`, `shaders/exposure.slang` and `shaders/display.slang`;
> - `demos/city-blocks/src/lib.rs` and `demos/city-plan/src/main.rs`;
> - `tools/map-check.sh`, D-022 and `docs/PROFILE.md`.

The question is how to make a dark room next to a sunlit opening read brighter in real time without blowing out the opening. It must work on NVIDIA and AMD, with no halos and no flicker, and without building on a live patent.

The short answer is a **local exposure in the display pass**:
- **How it works.** Each frame, a coarse bilateral grid of log luminance is built from the resolved HDR image and blurred. It is blended with a wide Gaussian of the same luminance. Each pixel then reads it at its own position and luminance. That gives a smooth exposure gain that pulls each region's base luminance towards middle grey.
- **Where it comes from.** The model is Durand and Dorsey's 2002 base/detail split on Chen, Paris and Durand's 2007 grid. Ghost of Tsushima shipped it (Patry 2021). Unreal Engine 5 exposes it as its "Bilateral" local exposure.
- **What it does to a hall.** With a shadow contrast of 0.7, a hall five stops under middle grey is lifted 1.5 stops. With a highlight contrast of 0.85, a doorway three stops over is lowered 0.45 stop.
- **Cost.** About 0.04–0.08 ms at 1600 × 900 on the 5070 Ti (an estimate) and about 2.4 MB at any resolution. It takes four small compute passes and a few taps in the existing display pass. Ghost of Tsushima measured 0.25 ms on a PS4 at 1080p.
- **Where it runs.** After TAA or DLAA, and outside their history. The histogram keeps metering the image before it. The grid is built and used in the same frame.
- **Patents.** The foundational patents have expired: MIT's US 7,146,059 for Durand and Dorsey, and the bilateral-grid family from NXP for unpaid fees. The live patents found nearby belong to camera pipelines (STMicroelectronics, Apple, Texas Instruments, GoPro, Irystec, Dolby, Samsung, Qualcomm). One of them, STMicroelectronics' US 11,756,172 (to 2041), claims applying a low-resolution log-luminance image of one frame to a later frame. Forge stays clear of that element by building and using the grid in the same frame. Under Forge's patent rule the owner is still asked before it is built.
- **Alternative.** Exposure fusion (Mertens et al. 2007, Wronski 2022, Unreal's "Fusion" method) avoids the grid's ringing, at more passes.

Part of #275 is metering by design: Forge keys the exposure on the brighter half of the pixels, so a sunlit doorway sets it. That is the case local exposure exists for.

> **State of the art in five sentences.**
> 1. Games lift interiors without blowing out windows by splitting the frame's log luminance into a smooth base and a detail layer and reducing only the base's contrast around middle grey. This is digital dodging and burning (Durand & Dorsey 2002; Reinhard et al. 2002).
> 2. The base must be edge-aware, or a bright doorway darkens a band of the room beside it (a halo). The real-time form is a coarse bilateral grid of log luminance: built by splatting, blurred, then read per pixel by trilinear interpolation at the pixel's own luminance (Chen, Paris & Durand 2007).
> 3. Ghost of Tsushima shipped one: a 64 × 32 × 64 grid blended 40/60 with a wide 2D Gaussian to hide the grid's ringing, at 0.25 ms on PS4 at 1080p (Patry 2021). Unreal Engine 5 exposes the same model with highlight and shadow contrast, detail strength, a blurred-luminance blend and a middle-grey bias, plus a second "Fusion" method (Epic's documentation).
> 4. Exposure fusion blends a few synthetic exposures of the frame by well-exposedness weights through Laplacian pyramids (Mertens, Kautz & Van Reeth 2007). It avoids the grid's ringing and gradient reversal, runs at quarter resolution with a guided-filter upsample, and is estimated at under 1 ms on the previous console generation (Wronski 2022, public-domain demo).
> 5. Both sit just before the display transform. Their foundational patents have expired, but camera makers hold many live local-tone-mapping patents. The details therefore matter: no reuse of one frame's low-resolution image on the next frame, no per-region tone curves, no analytic contrast-distortion curve.

**Contents**

1. [What #275 shows, in Forge's terms](#1-what-275-shows-in-forges-terms)
2. [Local exposure and local tone mapping in real time](#2-local-exposure-and-local-tone-mapping-in-real-time)
3. [Halos, gradient reversal and flicker](#3-halos-gradient-reversal-and-flicker)
4. [Order of operations: TAA, DLAA, the histogram, bloom, the display transform](#4-order-of-operations-taa-dlaa-the-histogram-bloom-the-display-transform)
5. [Cost and memory at 1600 × 900](#5-cost-and-memory-at-1600--900)
6. [Patents](#6-patents)
7. [Recommendation for Forge](#recommendation-for-forge)
8. [Checked and left out](#checked-and-left-out)
9. [Verification notes](#verification-notes)
10. [Sources](#sources)

---

## 1. What #275 shows, in Forge's terms

**How Forge meters.** Every pass draws pre-exposed: luminance in cd/m² times the frame's exposure, `1 / (1.2 · 2^EV100)` (D-022, `exposure.rs`).
- The meter counts the log2 luminance of the frame into 256 bins over [−16, 8] in pre-exposed units (`exposure.slang`).
- In the city and village demos the meter reads the jittered HDR frame, before TAA or DLAA (`taa_frame.color` in `city-blocks/src/lib.rs`).
- The CPU drops the black bin. It takes the log-average of the samples between the 50th and 98th percentiles as the key L.
- It moves EV100 towards `log2(8 L) − compensation`, at 1.5 per second towards darker and 0.8 per second towards brighter.

**Forge's middle grey.** From these two formulas, the key always lands at `exposure · L = 2^compensation / 9.6`. That is about 0.104 in pre-exposed units (log2 −3.26), whatever the scene. This note calls it **M**.

**Why the doorway sets the exposure.** The key is the log-average of the brighter half of the pixels. So any view where a sunlit doorway or window is a sizeable part of the frame is keyed mostly on the street. Unreal also weights its band towards the highlights: it suggests a Low Percent of "70 to 80" and a High Percent of "80 to 95" (Epic, Auto Exposure). The street sets the exposure, and the hall falls under it by the scene's own contrast.

**An illustration (assumed numbers, not a measurement).** Take a hall lit at 2 % of the street's illuminance, with walls of albedo 0.5 against a street of 0.3. It is log2(0.3 / (0.5 × 0.02)) ≈ 4.9 stops darker than the street. If the key sits a stop under the street, the hall sits about four stops under M. That is near the toe of every display curve Forge has.

**Why global exposure cannot fix both.** Lowering the key by n stops brightens the hall by n stops. It also pushes the doorway n stops further into the curve's shoulder.

**The photographic answer.** Dodging and burning brightens the dark region and darkens the bright one separately. Reinhard et al. (2002) build their operator on Adams' zone system and on dodging and burning. Unreal calls this *local exposure*; Wronski (2016, 2022) and Patry (2021) call it *local tone mapping*.

**What it does not change.** Local exposure changes how the frame is shown, not the light in it. Whether the hall's light itself is right (the probes, 2 m apart in the village since 2026-10-17; no lamps yet) is the question of `interior-lighting.md` (#270).

## 2. Local exposure and local tone mapping in real time

### 2.1 Base and detail: Durand and Dorsey 2002

Durand and Dorsey split the image into "a base layer, encoding large-scale variations", made with a bilateral filter, and a detail layer, the residual. "Only the base layer has its contrast reduced, thereby preserving detail" (project page, *read*).
- The FAQ's pseudocode works in log intensity, `log(base)=Bilateral(log(input intensity))`.
- It picks a compression factor so that the base reaches a target contrast. The author uses `log(5)`.
- A piecewise-linear approximation in intensity speeds the bilateral filter up by "two orders of magnitude".

Every method below shares this model: a gain set by the base, with the detail carried through. The method's patent, MIT's US 7,146,059 B1, has expired (§6).

### 2.2 Dodging and burning: Reinhard et al. 2002

Reinhard, Stark, Shirley and Ferwerda (ACM TOG 21(3), 2002) add a local "dodging and burning" step to a global operator.
- At each pixel it compares centre and surround Gaussians over a stack of scales.
- It keeps the largest scale without strong contrast, and divides the luminance by that local average.
- The constants come from memory and were not re-read, because the PDF could not be read as text: scales growing by a factor of 1.6, a threshold ε of about 0.05, a sharpening φ of 8, and a key of 0.18.

The scale selection limits halos without an edge-aware filter. But it evaluates a stack of Gaussians at full resolution, and no real-time game use was found. It is under [Checked and left out](#checked-and-left-out).

### 2.3 The bilateral grid: Chen, Paris and Durand 2007

Chen, Paris and Durand (ACM TOG 26(3), 103:1–103:9, SIGGRAPH 2007) store the image in a coarse 3D array over space and intensity.
1. **Splat.** Each pixel is added to the cell at its position and its own intensity, as a homogeneous pair (value × weight, weight).
2. **Blur.** The grid is blurred.
3. **Slice.** The result is read back per pixel by trilinear interpolation at the pixel's position and intensity, and the pair is divided.

Pixels of very different intensity land in different cells, so the blur does not mix them. The filter is edge-aware at the cost of a few small passes. Paris and Durand (2006, 2009) give the theory: the bilateral filter is a linear filter in the higher-dimensional space, and that space can be downsampled according to the filter's bandwidth.

What could and could not be checked:
- The project page (*read*) says the method runs on GPUs "to achieve real-time frame rates on high-definition video", with numbers for a GeForce 8800 GTX.
- The paper's slides (*read*, presenter notes only) show interactive local tone mapping of a 15-megapixel HDR panorama with Durand and Dorsey's method on the grid.
- A third-party summary says a 70 × 70 × 10 grid suffices for an 8-megapixel image (*search-recorded*, not checked against the paper).
- The paper's own sampling rates and timings could not be read: the PDF exceeded the fetch limit, and other copies were compressed.

On code: the page publishes code 0.1 with no licence stated. It withdrew version 0.2 "due to patent issues"; that version held a different technique, real-time video abstraction.

### 2.4 Ghost of Tsushima: the grid blended with a wide Gaussian (Patry 2021)

Patry's Advances 2021 talk (*read*) sets out the problem as Forge has it: "Dim interior lighting", "Large variation between interior and exterior luminance", "Maintain visibility while avoiding 'blow-outs'".

**The grid.**
- Size: "64 W x 32 H x 64 D (log luminance)". The range of the luminance axis is not given.
- Build: samples are added as homogeneous log-luminance values. Each sample is split between the two nearest luminance slices with linear weights. There are no X/Y weights, because the blur handles them.
- Blur: "Wide blur in X & Y, smaller in Z (or can omit)".
- Read: trilinear sampling and normalisation give the bilateral base B.

**The blend.** "We used 40% bilateral, 60% 2D Gaussian". The Gaussian is a radius-13 blur on the 64 × 32 image. It is "Upsampled to 256x128 using bicubic B-spline filtering", which "Avoids bilinear artifacts".

**The output.** In log luminance, `I_o = c×(B−M) + d×(I_i−B) + M`: c is the contrast scale, d the detail strength and M the "Midpoint log luminance". How M is chosen is not said. Typical in-game settings were 75 % contrast in high-contrast scenes.

**Artefacts.**
- Bilateral blurring of smooth gradients rings, on clouds, filtered shadows and specular highlights.
- Larger luminance buckets or a wider Z blur reduce the ringing but "Tends to increase halos".
- The wide Gaussian blend keeps halos from being noticeable.

**Cost.** "~250 us on PS4 at 1080p". The grid "Naturally fits alongside luminance histogram generation".

**Exposure.** Ghost of Tsushima's exposure is "primarily illuminance-based", using luminance "only if highlight luminance exceeds threshold". This breaks an exposure–albedo feedback loop.

### 2.5 Unreal Engine 5's Local Exposure

Epic's Auto Exposure page (UE 5.8, *read*) describes Local Exposure as a technique that "automatically applies local adjustments to exposure" on top of global exposure. It recommends setting it up when using Lumen GI. There are two methods:
- **Bilateral**: "decompose the image into base and detail layers using a bilateral blur… Only the base layer contrast is reduced in order to preserve detail".
- **Fusion**: "fuses multiple exposures according to quality measures".

The settings come from the documentation, the `FPostProcessSettings` API page (*read*) and the UE 5.0 Python API (*read*):

| Setting | What it does |
|---|---|
| Highlight Contrast, Shadow Contrast | Usually 0.6 to 1; API range 0–1. |
| Highlight and Shadow Contrast Curves | Contrast "based on the scene EV100". |
| Highlight and Shadow Threshold and Threshold Strength | Which regions count as highlights or shadows. |
| Detail Strength | "Values different than 1 enable local exposure"; "should be set to 1 in most cases"; API range 0–4. |
| Blurred Luminance Blend | "Blend between bilateral filtered and blurred luminance as the base layer. Blurred luminance helps preserve image appearance and specular highlights, and reduce ringing. Good values are usually in the range 0.4 .. 0.6". |
| Blurred Luminance Kernel Size Percent | Kernel size as a percentage of the screen. |
| Middle Grey Bias | Logarithmic: "0: no adjustment, -1:2x darker, -2:4x darker". |

UE 5.0 had a single Contrast Scale: "Value less than 1 will enable local exposure. Good values are usually in the range 0.6 .. 1.0". The defaults are neutral: in 2022, users found the settings had no effect until changed (forum thread, *read*). A Local Exposure visualisation mode shows where it acts.

**What Epic does not document.** Its cost, its grid size, and its place in the post chain.

**A third-party reading of the source.** The kansei project's pull request #49 (*read*) says it ports the Bilateral method "from the UE 5.8 source":
- grid cells of 64 × 64 half-resolution texels, with 32 bins of log2 luminance over −10 to 20;
- a blurred log luminance at 1/32 resolution, with a Gaussian kernel of 50 % of the screen;
- a blend of 0.6;
- the gain applied to the image the tonemapper receives, bloom included;
- its tonemap at 1080p took 0.46 ms with local exposure and 0.31 ms without (GPU not stated).

This is one author's reading of code under Epic's licence. Forge should take from it only the confirmation that UE ships Patry's published model. Forge must not port UE source.

**Reported artefacts.**
- UE-250274 (*search-recorded*; the page refused the fetch): translucency drawn after motion blur shows bright spots when Local Exposure and TSR are both on. Epic's suspected cause is that the local exposure pass runs at half resolution and misses those meshes.
- UE-327117 (forum thread, *read*): with the same final EV, local exposure lifted shadows differently depending on whether the EV came from compensation or from a locked range. An Epic engineer pointed at "the eye adaptation readback in darkly lit scenes". So UE couples local exposure and eye adaptation in some way that is not public.
- A 5.6 user removed "a black cloud artifact" by setting highlight contrast to 0.5 in the project defaults (*search-recorded*).

### 2.6 Exposure fusion: Mertens et al. 2007 and Wronski 2022

**Mertens et al.** Mertens, Kautz and Van Reeth (Pacific Graphics 2007, pp. 382–390; Computer Graphics Forum 2009) fuse a bracketed sequence straight into an LDR image, without building an HDR image (*search-recorded*: MathWorks' and IPOL's descriptions; the PDF was not readable as text).
- Each pixel is weighted by three measures: contrast (the Laplacian response), saturation (the standard deviation of R, G and B) and well-exposedness. Well-exposedness is a Gaussian around 0.5 with σ = 0.2, multiplied over the channels.
- Each measure is raised to an exponent, 1 by default.
- The weights are normalised. The images' Laplacian pyramids are blended with the weights' Gaussian pyramids, which avoids the seams of naive blending.

Hessel (IPOL 2018, *read*) publishes an implementation under BSD-2-Clause. He analyses its main limitation, "an out-of-range artifact".

**Wronski's real-time version.** Wronski (2022, *read*) turns it into a real-time local tone mapper:
- three synthetic exposures of the frame, "a pretty good starting choice";
- weights by exposure only: "I recommend using just the exposure";
- Laplacian blending down to a coarse level, "like on the 5th or 6th mip-map";
- all of it at ¼ × ¼ resolution, then a guided upsample to full resolution;
- cost: "it should definitely be under 1ms on even previous generation consoles", unprofiled. Tone mapping the synthetic exposures is the largest part, and a cheaper proxy curve is acceptable.

**His WebGL demo.** `main.js` (*read*) was released to the public domain: "I release it to public domain", in the post.
- Exposure 0.7, shadows +1.5 EV, highlights −2 EV. Each exposure goes through an ACES fit and a square root to give a lightness.
- Weights: `exp(-0.5 * diff * diff * sigmaSq)` around 0.5, with σ = 5.
- A pyramid built by 2 × 2 box downsampling. The base is blended at mip 6, and the Laplacians are added back down to mip 2.
- At mip 2, a 3 × 3 Gaussian-weighted local linear (guided) fit of the fused lightness against the middle exposure's lightness. The fit is then applied to the full-resolution lightness.
- The resulting ratio multiplies the linear HDR RGB before the final curve. It fades to 1 below a small luminance; the shader's own comment calls that fade a hack.

**Halos and edges in Wronski 2022.**
- Gaussian blending "generates pretty bad halos".
- Bilateral filtering brings "gradient reversals and edge ringing".
- Fusion limits each frequency's blend radius, so sharp edges get small blends.
- Guided upsampling "tends to blur out some edges".
- Pushed hard it still shows artefacts; more exposures help, at more cost.

The post says nothing about TAA or about the order relative to exposure. Unreal's "Fusion" method fuses "multiple exposures according to quality measures" (documentation). Which UE version added it was not found.

### 2.7 The guided filter (He et al. 2010, 2013)

He, Sun and Tang (ECCV 2010; TPAMI 35(6), 2013) define the output as a local linear function of a guide image. It is edge-preserving like the bilateral filter, with "better behaviors near edges", and has an exact linear-time algorithm whatever the kernel size (abstract, *search-recorded*). It is built into MATLAB (2014) and OpenCV 3.0 (project page, *read*). Wronski uses it to upsample the fused lightness. In Forge its role would be the same, an upsampler, not the base filter.

### 2.8 Other engines and games

- **God of War** (Santa Monica Studio). Wronski (2016, *read*) describes a shadows/highlights local exposure. Its luminance came from diffuse lighting only, "without albedo". It was "blurred bilaterally, clamped + modified to avoid extreme effects", at reduced resolution. Halos appeared at strong settings, from working at lower resolution. Guerrilla Games is credited with "an example of how to apply it in runtime". The 2022 post calls it a hacked version whose halos showed when pushed.
- **Ghost of Tsushima**: §2.4.
- **Unreal Engine 5**: §2.5.
- **Unity** has a roadmap entry, "Local Exposure", marked Under Consideration and dated 2 April 2025. It targets open worlds where frame-based auto exposure over- or under-exposes high-dynamic-range lighting (*search-recorded*; the page returned 404 when fetched). HDRP's documented exposure is global.
- **Godot 4** documents its post chain as depth of field, glow, tonemap with auto exposure, then adjustments. No local exposure was found (*search-recorded*).
- **Frostbite** (Fry 2017's HDR talk), **Call of Duty** and **The Last of Us**: no source found that says they ship local exposure or local tone mapping. This is unverified either way.

## 3. Halos, gradient reversal and flicker

| Base layer | Halos | Ringing, gradient reversal | Under motion | How shipped versions handle it |
|---|---|---|---|---|
| Gaussian blur of luminance | Yes: bright regions leak into darker neighbours (Wronski 2022) | None | Halos "visible and distracting" with a moving camera (Wronski 2022) | God of War limited strength and clamped (Wronski 2016) |
| Bilateral grid | None across edges wider than its luminance bins | Ringing on smooth gradients: clouds, filtered shadows, specular highlights (Patry 2021); gradient reversal (Wronski 2022) | Not documented | Blended with a wide Gaussian, 40/60 (Patry) or 0.4–0.6 (Epic) |
| Exposure fusion | Small at sharp edges, since the blend radius follows frequency (Wronski 2022) | Not reported; an out-of-range artefact (Hessel 2018) | Simple bilinear downsampling "would alias and flicker" (Wronski, about his demo) | ¼ resolution, guided upsample, more exposures |

**Why the Gaussian blend works.** The halo it brings back is as wide as its kernel: radius 13 on a 64-wide image, about a fifth of the screen. It reads as a soft gradient across the room, not an outline along the door frame. This is an inference from Patry's numbers, not his words. Patry says only that the blend kept haloing from being noticeable.

**Flicker in general.** Reviews of video tone mapping treat temporal coherence (flicker, ghosting) as a primary criterion (Eilertsen, Mantiuk & Unger 2017, abstract; *search-recorded*).

**Flicker in the grid.** Three things can change from frame to frame, each with its answer:
1. **Luminance slices.** A pixel moves between luminance slices continuously, because the luminance splat is linear.
2. **Cell borders.** Without X/Y splat weights, a pixel's contribution jumps from one cell to the next as it crosses a border. The grid's blur and the trilinear read smooth the jump; Patry relies on that. Forge's flicker measurement (Recommendation) decides whether X/Y weights are needed.
3. **The guide.** The grid is read at the pixel's own luminance, so the gain follows that luminance. An aliased guide gives an aliased gain. Building the grid from the resolved image and reading it with the resolved image makes the guide as stable as the shown frame.

**The gain has no adaptation.** The global exposure adapts at 0.8–1.5 per second. The local gain follows the content in the same frame. Under continuous camera motion the content changes continuously, and so does the gain. A doorway that enters the view quickly changes it quickly. The way-in walk checks this.

## 4. Order of operations: TAA, DLAA, the histogram, bloom, the display transform

Today the city and village frame runs in this order (`city-blocks/src/lib.rs`). The first five steps stay as they are:
1. The scene, jittered and pre-exposed.
2. The histogram of the jittered frame, for the frames to come.
3. TAA's `resolve_history`, or DLAA's `upscale`.
4. Bloom, from the resolved image.
5. `taa.show` or `Display::draw`. Both run the display transform of `tonemap.slang`, with bloom mixed in first (`display.slang`).

Local exposure adds two things:
- **New pass:** the grid, built from the resolved image after step 3.
- **New steps in step 5:** after bloom is mixed in, the local gain is applied, then the display transform and the encoding (SDR, HDR10 or scRGB).

**Why after the resolve:**
- The TAA history is rescaled by a single ratio of exposures (D-022). A spatial gain inside the history would be wrong as soon as the gain field moved.
- DLSS takes one exposure value (D-045).
- The gain field is smooth, so it needs no anti-aliasing.
- The guide must be the stable, resolved luminance (§3).

Patry places the grid in the tonemapping stage. Wronski ends with "Transfer the lightness to the full target image", followed by tone mapping and grading. The third-party reading of UE puts the gain in the tonemapper.

**The histogram should not meter the locally exposed image.** The local gain is anchored on M, and M is where the global exposure puts the key. A meter that saw the gained image would read a key already pulled towards M. The global exposure would then correct less than the scene asks: a feedback loop with nothing to gain. This is my derivation, not a source's.
- Keep metering the scene before local exposure. The grid and the histogram can later share one read of the image (Patry).
- The HDR content-light meter (#125) measures the frame as shown, after local exposure. That is right for MaxCLL and MaxFALL.
- Unreal couples the two in a way that is not public, and that coupling produced a reported inconsistency (UE-327117).

**The midpoint.** Forge's middle grey is fixed in pre-exposed units: M = log2(2^compensation / 9.6), plus the bias.
- **Anchoring on this constant** rather than on the current frame's key makes the local gain act at once when walking in. The hall, below M, is lifted while the global exposure is still adapting. The lift fades as the global exposure catches up. Walking out, the street is above M and is darkened at once.
- **At night** (D-046), the CPU darkens the exposure by up to `night_stops`. M must include that offset, or local exposure would lift the night back up.

**HDR output.** The gain sits in the same place on every output, before the encoding. The HDR presets have headroom above paper white, so highlights could be compressed less there; that is later work.

## 5. Cost and memory at 1600 × 900

All numbers below are estimates for the recommended design, from Forge's measured neighbours: the exposure histogram 0.015–0.02 ms, `post/display transform` 0.011 ms (0.020 with bloom), `post/bloom` 0.041 ms (`PROFILE.md`, D-022). The F1 overlay replaces them.

| Pass | Work | Memory | Estimate on the 5070 Ti |
|---|---|---|---|
| Grid | One read of the resolved image (1.44 M pixels × 8 B = 11.5 MB). One 16 × 16 group per cell; atomics on shared memory only. | 64 × 36 × 64 cells × 8 B (RGBA16F) = 1.18 MB, plus a 64 × 36 mean | 0.02–0.04 ms |
| Grid blur | X, Y and luminance passes, ping-ponging two grids | +1.18 MB | 0.01–0.02 ms (dispatch-bound) |
| 2D Gaussian | One group holds the 64 × 36 image (9 KB in, 9 KB out) | 2 × 4.6 KB (R16F) | ~0.003 ms |
| Apply | Inside the display pass: one trilinear 3D tap, four bilinear taps (bicubic B-spline), ~25 ALU | — | +0.005–0.015 ms |
| **Total** | | **~2.4 MB at any resolution** | **0.04–0.08 ms** |

Points of comparison:
- Ghost of Tsushima measured 0.25 ms on PS4 at 1080p (Patry 2021).
- The kansei port measured +0.15 ms at 1080p on a GPU it does not name.
- The read of the full image is the bandwidth floor: 11.5 MB at 896 GB/s (the 5070 Ti's bandwidth, from memory) is 0.013 ms. Building from half resolution, as UE does by the third-party reading, would not be worth it here.

Exposure fusion would need more work:
- three exposures through a proxy curve at 400 × 225;
- two pyramids of about six levels each;
- six blend levels and a guided upsample;
- about 15–20 dispatches in all.

That is an estimated 0.05–0.15 ms, mostly in dispatches. Wronski's own estimate is under 1 ms on the previous consoles.

**Cross-vendor notes.**
- Groupshared use is 512 B per grid group, and 18 KB for the 2D Gaussian: under 32 KB.
- No subgroup operations and no float atomics: sums are fixed-point `uint`, as the histogram counts.
- RGBA16F is a mandatory storage format in Vulkan, and linear filtering of a 3D image is supported on both vendors.

## 6. Patents

The searches covered Dolby, Apple, Samsung, Google, Qualcomm, Adobe, Epic, Sony, MIT and Microsoft, for local tone mapping, bilateral-grid tone mapping and exposure fusion. They used a general web search plus Google Patents record pages. Google Patents' own search page is drawn by script and could not be read. Justia and FreePatentsOnline refused the fetch (403). **This is not a professional search.**

**Behind the recommended method:**

| Technique | Patent | Holder | Jurisdiction | Status, expiry | Bearing |
|---|---|---|---|---|---|
| Bilateral filter for HDR display (Durand & Dorsey 2002) | US 7,146,059 B1 | MIT | US | Expired ("Expired – Lifetime"; adjusted expiry 2025-02-08) | Free to use. |
| Bilateral grid by integration and box sums | US 8,731,320 B2; US 9,047,656 B2; US 9,552,631 B2 (WO 2010/084460, EP priority 2009-01-20) | Filed by an individual (NXP/Trident line); Google lists Dynamic Data Technologies | US (WO ceased; EP not checked) | All three expired, fee-related; 8,731,320's legal events record expiry on 2022-06-27 | Free to use. |
| Bilateral grid (Chen et al. 2007), fast bilateral approximation (Paris & Durand 2006) | None found in MIT's or Adobe's name | — | — | — | No patent found. |
| Fast bilateral approximation (unrelated method) | US 2011/0200269 A1 | Seiko Epson | US | Abandoned 2012, never granted | None. |
| Patry's blend (Ghost of Tsushima), Unreal's local exposure | None found for Sony or Epic | — | — | — | No patent found. |

**Live patents near the recommended method** (claims summarised from their pages; where the claims were not read, it says so):

| Patent | Holder | Jurisdiction | Status, expiry | Bearing on Forge |
|---|---|---|---|---|
| US 11,756,172 B2, "Local tone mapping for HDR video" | STMicroelectronics | US | Active; adjusted expiry 2041-06-23; priority 2020-02-25 | **Nearest.** Claim 1: split a first image into sub-blocks, take logs, store a "mini-image" of sub-block log intensities, then modify a *second* image of the sequence with a gain from a global strength and a local contrast drawn from the second image's logs and the first image's mini-image. Claim 2 adds spatial and intensity weights. Forge builds and uses its grid in the same frame and keeps no grid history; using a grid on a later frame would come near claim 1. Ask the owner. |
| US 9,105,078 B2, "Systems and methods for local tone mapping" | Apple | US | Active per Google; adjusted expiry 2033-04-06 | Claims not read (page truncated). The search record describes image-signal-processor (ISP) logic that applies spatially varying tone curves through a grid of lookup tables indexed by luminance, with spatially varying colour correction. Forge uses one curve and a per-pixel exposure gain. Its sibling US 9,741,099 B2 has expired (fees). |
| US 9,437,171 B2 | Texas Instruments | US | Active; adjusted expiry 2034-05-22 | One tone curve per overlapping region, blended per pixel. Forge builds no per-region curves. |
| US 10,284,785 B2 and continuations 10,554,903; 11,178,341; 11,800,238; 12,489,984; also US 10,878,541 B2 ("object aware") | GoPro | US | Active; 2037–2038 | Claim 1 is a system with an image sensor: a bilateral-filter split, detail weighted more, a tone mapping, then a second split in a gamma "perceptual domain". Claim 11 is the same chain as a method. Claim 14 derives per-region gains from a histogram of the low-frequency image. Forge has no sensor and no perceptual second stage, and its gain comes from contrast scales, not a histogram of the base. Detail strength above 1 is one claimed element; it stays at 1 in version 1. |
| US 11,107,204 B2 (to 2038-05-04), US 11,756,174 B2 (to 2036-12-14), JP 7008621 B2 (to 2036-09-02) | Faurecia Irystec | US, JP | Active | Eilertsen, Mantiuk and Unger's real-time tone mapping. Every independent claim seen requires a tone curve determined from a model of contrast distortion and computed analytically. Claim 18 adds a base/detail split around that curve. Forge's curves are fixed and its contrast scales are constants. Do not build that operator. |
| US 9,501,818 B2 (parent US 9,299,317; EP 2 681 710) | Dolby | US, EP | Active; anticipated expiry 2032-03-01 | Ward's local multiscale operator: a global operator's ratio image plus ratio images at several resolutions, merged with weights. Not used. |
| US 12,243,201 B2 | Samsung | US | Active; to 2043 | Joint camera auto-exposure and tone mapping from sensor statistics. Not near. |
| US 11,769,234 B2 | Qualcomm | US | Active; to 2041 | Recomputes a histogram-based tone mapping only when a perceptual change exceeds a threshold. Not near. |

**Near the exposure-fusion alternative:**
- **US 11,430,094 B2**, "Guided multi-exposure image fusion", Samsung, US. Active; adjusted expiry 2041-01-15. Images at different exposure values, each colour channel split into base and detail, weight maps per layer made with guided filters. Claim 1 was not read. Ask the owner before taking the fusion route with a guided upsample.
- **Mertens et al. 2007**: no patent found.
- **Guided filter** (He et al.): no Microsoft patent found. Thomson's US 9,286,663 B2 (filtering with a guidance image) lapsed for fees in 2020.

**Not found:** Google (HDR+, HDRNet), Adobe (local Laplacian filters, the bilateral grid), Epic and Sony. Apple's US 9,129,388 B2 (a global approximation of local operators) lapsed in 2019.

**Unpatented or expired route.** Durand and Dorsey's base/detail model (expired), on Chen et al.'s grid (no patent found, and the NXP-line grid patents expired), with Patry's published Gaussian blend (no patent found), built and used within one frame.

## Recommendation for Forge

### The method

Use a **bilateral-grid local exposure** after Patry 2021 and Chen et al. 2007, applied as a per-pixel exposure gain just before the display transform. Exposure fusion is the second method, only if the grid's ringing shows in the checks below. Build after the owner's go: the method is not built on a live patent found here, but STMicroelectronics' US 11,756,172 is close enough that the owner should know before it is built.

In log2 pre-exposed luminance, at each pixel:

```
l     = log2(max(Y, 2^-16))              Y: Rec.709 luminance of the resolved pixel
B     = lerp(B_grid(u, v, l), B_blur(u, v), blend)
c     = B < M ? shadow_contrast : highlight_contrast
gain  = exp2((c − 1)·(B − M) + (d − 1)·(l − B))     d = 1 in version 1
gain  = clamp(gain, 2^-max_darken, 2^max_lift), faded to 1 below M − 12 EV
rgb  *= gain                              after bloom is mixed, before the curve
M     = log2(2^compensation / 9.6) + bias − night offset   (§4)
```

With d = 1 the gain depends only on the base, so it is a spatially varying exposure. Detail passes through untouched.

### The passes

1. **`post/local exposure grid`** (compute)
   - Reads the resolved HDR image: TAA's history or DLAA's output.
   - The grid is 64 × 36 cells over the screen and 64 slices of log2 luminance over [−16, 8], the histogram's range (0.375 EV per slice). Cells are 25 × 25 px at 1600 × 900, 30 at 1080p and 40 at 1440p. The SSAA frame gets the same look.
   - One 16 × 16 group per cell loops over the cell's pixels. It splats (l, 1) linearly into the two nearest slices, with fixed-point `uint` atomics in groupshared memory. The scales are set so a 64 × 64-pixel cell cannot overflow.
   - It writes the column as (Σw·l / N, Σw / N), where N is the cell's pixel count, into an RGBA16F 3D image. It writes the cell's mean l into a 64 × 36 R16F image.
   - Each group owns its cell, so there are no global atomics and no X/Y splat weights (Patry).
2. **`post/local exposure blur`** (compute, four dispatches)
   - Grid: a Gaussian in X, then in Y (σ = 1 cell to start), then a [1 2 1]/4 tent in luminance, ping-ponging two 3D images.
   - Cell means: a 2D Gaussian of radius 13 cells on 64 (Patry), done by one group in shared memory.
3. **Apply** (in `tonemap.slang`, called by `taa.show` and `Display::draw`)
   - Read the grid trilinearly at (u, v, slice(l)). B_grid = x / max(y, ε). Where the weight y is small, fade to B_blur.
   - Read B_blur with a bicubic B-spline from four bilinear taps, as Patry did to avoid bilinear artefacts.
   - Then apply the formula above.

Every pass declares its graph accesses and is marked for the F1 overlay. Nothing changes upstream: the scene, the histogram, TAA, DLSS and bloom are as they are.

### Parameters, defaults and controls

| Parameter | Default | Source of the default |
|---|---|---|
| `shadow_contrast` | 0.7 | Epic: 0.6–1; Patry: 75 % in high-contrast scenes |
| `highlight_contrast` | 0.85 | Same range; a lighter touch on the street |
| `detail` | 1.0, fixed in version 1 | Epic: "should be set to 1 in most cases" |
| `blend` | 0.6 | Patry: 60 % Gaussian; Epic: 0.4–0.6 |
| `blur_radius` | 13 cells of 64 | Patry |
| `middle_grey_bias` | 0 EV | Epic's control, in EV |
| `max_lift`, `max_darken` | 3 EV, 2 EV | Forge's safety clamp (God of War clamped its gain, Wronski 2016) |
| dark floor | No lift below M − 12 EV, faded over 2 EV | Keeps black black: the ballad's space, and noise in deep shadow. Wronski's demo fades its gain near black. |

Controls:
- `--local-exposure on|off`;
- `--local-exposure-contrast S,H`;
- a toggle key (a free one);
- a line in the F1 overlay with on or off and the values;
- `--local-exposure-view`, a false-colour map of log2 gain: blue for lift, red for darkening.

The feature is **off by default** until the owner has seen it. After that it is on in `city-plan --village` and `city-blocks`, and `map-check --accept` makes the new set the accepted one. Planets, asteroids and the ballad stay off unless asked.

### How to verify it

1. **Unit tests on the CPU mirror** of the read and the gain:
   - c = 1 and d = 1 give a gain of exactly 1;
   - two flat regions give each region its own mean as the base, away from the edge, and gains within 0.01 EV of (1 − c)(M − B);
   - a step edge on the bilateral-only path gives no gain gradient beyond one cell from the edge;
   - a ramp plus a step stays monotonic.
   - The GPU grid is also compared with the CPU grid on one captured HDR frame.
2. **Images.**
   - The map check's `hall`, `landing` and `room` views and the "way in" walk (`city-plan --village`).
   - A new **`door`** view, from the back of the hall looking at the open front door. That is #275's case: `hall` looks away from the door.
   - A sunlit street at noon, where the gain should be near 0 EV.
   - A low sun with a sky gradient, to look for ringing.
   - The `city-blocks` bench.
3. **Metrics** (the thresholds are starting points, calibrated on the first run):
   - *Contrast moved as intended.* Using hall and doorway masks from the global frame, compare each region's mean log2 luminance with the feature off and on. The change should be (1 − c)(M − B) within 0.1 EV.
   - *Halos, with ꟻLIP against the global-exposure frame.* Run `imgdiff global.png local.png --flip-map map.png` on the same seed and frame. A uniform brightening shows as a flat error inside each region; a halo shows as a band along the door frame. Measure the mean ꟻLIP in a band 0–32 px from the door's edge against the mean 64–256 px into the hall, on each side. Start at a ratio of 1.3 or less with blend 0.6; the bilateral-only path should be about 1. Cross-check on the gain map along a scanline through the edge.
   - *Gradient reversal.* On the sky view, count pixels where |Δ log2 L| > 0.05 EV per pixel and the sign of the difference flips between the two frames. The target is 0.
   - *Flicker.* A still camera under `--dlaa` with `--fixed-step --capture-every 10`, consecutive frames compared by imgdiff. With the feature on, the count must stay within the off run's plus a small margin; the owner's default is 0.05 %. A turn and a `--dolly` with `imgdiff --then`. On the way-in walk, plot the mean log2 luminance per frame, on against off: no frame-to-frame step above 0.05 EV that the off run lacks.
   - *Cost.* F1 zones, best of three, at 1600 × 900 and 1440p. The budget is 0.1 ms.
   - *AMD.* One `tools/amd-check.sh` run.
4. **The owner's look.** A contact sheet of the views with the feature off and with it on at two settings.

### What can stay out of the first version

- the Fusion method (§2.6);
- contrast curves by EV100, and highlight and shadow thresholds (Unreal);
- detail strength other than 1, a "clarity" control that also boosts aliasing and noise;
- any temporal filtering of the grid, which is not needed so far and sits near US 11,756,172;
- X/Y splat weights, added only if the flicker check asks for them;
- a guide from diffuse light without albedo (God of War) and illuminance metering (Ghost of Tsushima);
- separate defaults for HDR output;
- sharing one read with the histogram;
- per-area settings (volumes).

## Checked and left out

- **Reinhard et al. 2002, local operator.** A stack of centre-surround Gaussians per pixel at full resolution. No real-time game use found.
- **A full-resolution bilateral filter** (Durand & Dorsey, with its approximation). The grid does the same at a fraction of the cost.
- **Local Laplacian filters** (Paris, Hasinoff & Kautz 2011; Aubry et al. 2014). Free of halos, but they need a pyramid per intensity sample, which is heavier than either method above. Adobe's patent position was not determined. A UK impact case study ties the method to Lightroom's local filtering (*search-recorded*).
- **HDRNet** (Gharbi et al. 2017). It learns a bilateral grid of affine coefficients, so it needs a network and training data.
- **The guided filter as the base filter.** Not found in a shipped game. Kept as the upsampler of the fusion alternative.
- **Per-region tone curves** (adaptive-histogram style). Live patents: TI US 9,437,171 and Apple US 9,105,078.
- **Irystec's real-time noise-aware operator** (live patents) and **Dolby's multiscale ratio operator** (live patent).
- **Changing the metering instead.** Lowering the 50–98 % band, centre weighting (Unreal's metering mask "helps stabilize auto exposure"), or illuminance metering (Patry). These are separate exposure decisions. They move both regions together and do not solve #275 alone.

## Verification notes

**Read on 2026-10-17:**
- Epic: the Auto Exposure page (UE 5.8), the `FPostProcessSettings` API page, the 5.0 Python API page, and the forum threads 548130 and 2670675.
- Patry's Advances 2021 deck (HTML).
- Wronski's 2022 and 2016 posts, and the demo's `main.js`.
- The bilateral-grid project page, Adobe Research's page for it, and the paper's slides (presenter notes only).
- Durand's 2002 project page and FAQ; the guided-filter project page; IPOL 2018/230; kansei PR #49.
- Google Patents pages for US 8,731,320, 9,501,818, 9,741,099, 11,107,204 (claims included), 11,756,174, JP 7008621, 9,129,388, 9,105,078 (metadata only), 11,769,234, 11,430,094 (metadata and abstract), 9,437,171, 11,756,172 (claims included), 12,243,201, 10,284,785 (claims included), 10,878,541, 7,146,059, 9,286,663 and 2011/0200269.
- The USPTO gazette page for 12,489,984.
- Forge's own files, as listed in the header.

**Search-recorded only:**
- UE-250274 (403) and the 5.6 "black cloud" forum post.
- Unity roadmap 1899 (404 on fetch) and the Godot documentation.
- The Mertens et al. parameters (from MathWorks and IPOL extracts) and the He et al. abstract.
- The Eilertsen et al. 2017 abstract; Fry 2017; local Laplacian filters; HDRNet; Paris & Durand 2006.
- The 70 × 70 × 10 grid figure.
- Dolby's EP 2 681 710.
- Apple US 9,105,078's claim content.

**From memory, not re-read:** Reinhard's constants (1.6, ε 0.05, φ 8, key 0.18) and the 5070 Ti's 896 GB/s.

**Could not be verified:**
- Chen et al.'s own sampling rates and timings (PDF too large or compressed).
- Unreal's grid size, cost, position in the post chain, defaults, and the version that added Fusion. There is only a third-party reading of Epic's source.
- Whether Frostbite, Call of Duty or The Last of Us use local exposure.
- Every claim not read in full.

**Derived or estimated:**
- Forge's middle grey (log2 −3.26), derived from `exposure.rs`.
- The hall illustration, which uses assumed numbers.
- Every cost in §5, which is an estimate until the F1 overlay measures it.

## Sources

- **Frédo Durand, Julie Dorsey. "Fast Bilateral Filtering for the Display of High-Dynamic-Range Images." SIGGRAPH 2002.** [paper] https://people.csail.mit.edu/fredo/PUBLI/Siggraph2002/
- **Erik Reinhard, Michael Stark, Peter Shirley, James Ferwerda. "Photographic Tone Reproduction for Digital Images." ACM TOG 21(3), 2002 (tech report UUCS-02-001).** [paper] https://users.cs.utah.edu/docs/techreports/2002/pdf/UUCS-02-001.pdf
- **Jiawen Chen, Sylvain Paris, Frédo Durand. "Real-time Edge-Aware Image Processing with the Bilateral Grid." ACM TOG 26(3), SIGGRAPH 2007 (code 0.1, licence not stated).** [paper] [code] https://groups.csail.mit.edu/graphics/bilagrid/
- **Sylvain Paris, Frédo Durand. "A Fast Approximation of the Bilateral Filter using a Signal Processing Approach." ECCV 2006; IJCV 2009.** [paper] https://people.csail.mit.edu/sparis/publi/2009/ijcv/Paris_09_Fast_Approximation.pdf
- **Jasmin Patry. "Real-Time Samurai Cinema: Lighting, Atmosphere, and Tonemapping in Ghost of Tsushima." SIGGRAPH 2021 Advances in Real-Time Rendering.** [talk] [recent] https://advances.realtimerendering.com/s2021/jpatry_advances2021/index.html
- **Epic Games. "Auto Exposure in Unreal Engine." Unreal Engine 5.8 documentation.** [docs] [recent] https://dev.epicgames.com/documentation/en-us/unreal-engine/auto-exposure-in-unreal-engine
- **Epic Games. "FPostProcessSettings." Unreal Engine API reference.** [docs] [recent] https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Engine/FPostProcessSettings
- **Epic Games. "unreal.PostProcessSettings." Unreal Python API, 5.0.** [docs] https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/PostProcessSettings?application_version=5.0
- **Epic Developer Community. "Is Local exposure available in unreal 5.0?" Forum, 2022.** [web] https://forums.unrealengine.com/t/is-local-exposure-available-in-unreal-5-0/548130
- **Epic Developer Community. "inconsistent Local Exposure behaviour" (UE-327117). Forum.** [web] [recent] https://forums.unrealengine.com/t/inconsistent-local-exposure-behaviour/2670675
- **Epic Games. UE-250274 (Local Exposure, TSR and translucency after motion blur). Issue tracker.** [web] [recent] https://issues.unrealengine.com/issue/UE-250274
- **Siroko. "feat(tonemap): Unreal Engine 5's local exposure, its bilateral method." kansei pull request #49 (licence not stated).** [code] [recent] https://github.com/Siroko/kansei/pull/49
- **Tom Mertens, Jan Kautz, Frank Van Reeth. "Exposure Fusion." Pacific Graphics 2007; "Exposure Fusion: A Simple and Practical Alternative to High Dynamic Range Photography." CGF 28(1), 2009.** [paper] https://www.cs.princeton.edu/courses/archive/spring14/cos426/papers/Mertens07.pdf
- **Charles Hessel. "An Implementation of the Exposure Fusion Algorithm." IPOL 8, 2018 (BSD-2-Clause).** [paper] [code] https://ipol.im/pub/art/2018/230
- **Bartlomiej Wronski. "Exposure Fusion – local tonemapping for real-time rendering." Blog, 28 February 2022.** [web] [recent] https://bartwronski.com/2022/02/28/exposure-fusion-local-tonemapping-for-real-time-rendering/
- **Bartlomiej Wronski. Local tonemapping WebGL demo and `main.js`, 2022 (public domain, per the post).** [code] [recent] https://bartwronski.github.io/local_tonemapping_js_demo/
- **Bartlomiej Wronski. "Localized tonemapping – is global exposure and global tonemapping operator enough for video games?" Blog, 29 August 2016.** [web] https://bartwronski.com/2016/08/29/localized-tonemapping/
- **Bartlomiej Wronski. "Local linear models and guided filtering." Blog, 22 September 2019 (linked from the 2022 post, not read).** [web] https://bartwronski.wordpress.com/2019/09/22/local-linear-models-guided-filter/
- **Kaiming He, Jian Sun, Xiaoou Tang. "Guided Image Filtering." ECCV 2010; TPAMI 35(6), 2013.** [paper] [code] https://people.csail.mit.edu/kaiming/eccv10/index.html
- **Sylvain Paris, Samuel Hasinoff, Jan Kautz. "Local Laplacian Filters: Edge-aware Image Processing with a Laplacian Pyramid." SIGGRAPH 2011.** [paper] https://people.csail.mit.edu/sparis/publi/2011/siggraph/
- **Mathieu Aubry, Sylvain Paris, Samuel Hasinoff, Jan Kautz, Frédo Durand. "Fast Local Laplacian Filters: Theory and Applications." ACM TOG 2014.** [paper] https://jankautz.com/publications/FastLLF_TOG14.pdf
- **Michaël Gharbi, Jiawen Chen, Jonathan Barron, Samuel Hasinoff, Frédo Durand. "Deep Bilateral Learning for Real-Time Image Enhancement." SIGGRAPH 2017.** [paper] https://groups.csail.mit.edu/graphics/hdrnet/
- **Gabriel Eilertsen, Rafał Mantiuk, Jonas Unger. "A Comparative Review of Tone-mapping Algorithms for High Dynamic Range Video." CGF 36(2), Eurographics 2017 STAR.** [paper] https://www.cl.cam.ac.uk/~rkm38/pdfs/eilertsen2017tmo_star.pdf
- **Unity Technologies. "Local Exposure." Roadmap entry, Under Consideration, 2 April 2025.** [web] [recent] https://legacy-activation.unity3d.com/roadmap/1899-local-exposure
- **Godot Engine. "Environment and post-processing." Documentation (stable).** [docs] https://docs.godotengine.org/en/stable/tutorials/3d/environment_and_post_processing.html
- **Alex Fry. "High Dynamic Range color grading and display in Frostbite." GDC 2017.** [talk] https://www.ea.com/news/high-dynamic-range-color-grading-and-display-in-frostbite
- **Krzysztof Narkowicz. "Automatic Exposure." Blog, 9 January 2016 (Forge's metering; not re-read).** [web] https://knarkowicz.wordpress.com/2016/01/09/automatic-exposure/
- **NVIDIA. ꟻLIP (NVlabs/flip), as used by `imgdiff` (not re-read for this note).** [code] https://github.com/NVlabs/flip
- **MIT (Durand, Dorsey). "Method of performing fast bilateral filtering and using the same for the display of high-dynamic-range images." US 7,146,059 B1, granted 2006-12-05 (expired).** [patent] https://patents.google.com/patent/US7146059B1
- **Andre Leitao (Dynamic Data Technologies per Google). "Image processing using a bilateral grid." US 8,731,320 B2, with US 9,047,656 B2 and US 9,552,631 B2 (all expired, fees).** [patent] https://patents.google.com/patent/US8731320B2/en
- **Seiko Epson. "Fast Approximation For Bilateral Filter." US 2011/0200269 A1 (abandoned).** [patent] https://patents.google.com/patent/US20110200269A1/en
- **STMicroelectronics. "Local tone mapping for HDR video." US 11,756,172 B2, granted 2023-09-12 (active, to 2041).** [patent] [recent] https://patents.google.com/patent/US11756172B2/en
- **Apple. "Systems and methods for local tone mapping." US 9,105,078 B2, granted 2015-08-11 (active, to 2033).** [patent] https://patents.google.com/patent/US9105078B2/en
- **Apple. "Systems and methods for local tone mapping." US 9,741,099 B2, granted 2017-08-22 (expired, fees).** [patent] https://patents.google.com/patent/US9741099B2/en
- **Apple. "Global approximation to spatially varying tone mapping operators." US 9,129,388 B2 (lapsed 2019).** [patent] https://patents.google.com/patent/US9129388B2/en
- **Texas Instruments. "Local tone mapping for high dynamic range images." US 9,437,171 B2, granted 2016-09-06 (active, to 2034).** [patent] https://patents.google.com/patent/US9437171
- **GoPro. "Local tone mapping." US 10,284,785 B2, granted 2019-05-07 (active, to 2037), and continuations to US 12,489,984 B2.** [patent] https://patents.google.com/patent/US10284785B2/en
- **GoPro. "Local tone mapping." US 12,489,984 B2, Official Gazette, 2 December 2025.** [patent] [recent] https://patentsgazette.uspto.gov/week48/OG/html/1541-1/US12489984-20251202.html
- **GoPro. "Object aware local tone mapping." US 10,878,541 B2, granted 2020-12-29 (active, to 2038).** [patent] https://patents.google.com/patent/US10878541B2/en
- **Faurecia Irystec (Unger, Eilertsen, Mantiuk). "System and method for real-time tone-mapping." US 11,107,204 B2 (active, to 2038); US 11,756,174 B2 (to 2036); JP 7008621 B2 (to 2036).** [patent] https://patents.google.com/patent/US11107204B2/en
- **Dolby (Gregory Ward). "Local multiscale tone-mapping operator." US 9,501,818 B2, granted 2016-11-22 (active, to 2032).** [patent] https://patents.google.com/patent/US9501818B2/en
- **Samsung. "Guided multi-exposure image fusion." US 11,430,094 B2, granted 2022-08-30 (active, to 2041).** [patent] [recent] https://patents.google.com/patent/US11430094B2/en
- **Samsung. "Joint auto-exposure tone-mapping system." US 12,243,201 B2, granted 2025-03-04 (active, to 2043).** [patent] [recent] https://patents.google.com/patent/US12243201B2/en
- **Qualcomm. "Methods and apparatus for histogram based tone mapping." US 11,769,234 B2, granted 2023-09-26 (active, to 2041).** [patent] [recent] https://patents.google.com/patent/US11769234B2/en
- **Thomson Licensing. "Method and apparatus for filtering an image using a guidance image." US 9,286,663 B2 (lapsed 2020).** [patent] https://patents.google.com/patent/US9286663B2/en
