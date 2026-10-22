# Coarse and fine probe levels indoors (#353): one light per room at any distance

> Written 2026-10-23 for #353 by a research agent, with the owner's go. It is the companion to `probe-leaks-indoors.md` (D-068), `lighting-patents.md` and `lighting-gi.md`.
> - Each source is marked *read* (fetched that day), *search-recorded* (seen only in a search engine's record) or *prior note* (read in an earlier Forge note).
> - Forge's facts come from `shaders/probes.slang`, `shaders/probe_update.slang`, `crates/forge-render/src/probes.rs` and #353.
> - Numbers marked *derived* are arithmetic or assumptions, not measurements. This is not legal advice.
>
> **Measured after writing** (2026-10-23, the village room of #353, ebbd2075; levels are mean sRGB luma):
> - **Recommendation 2, linear blending in room lookups** (surfaces and hits). It brightens rooms 2–4 levels under both cascades. The 2 m cascade's gap to the 1 m one narrows by about 1.5 levels: back wall 8.5 → 7.0, the wall by the window 10.0 → 7.8.
> - **Recommendation 3's per-cascade counts** are built (d22d3779, `--probe-finest`). In the village, serially, the finest cascade at 32 across costs +0.21–0.32 ms, and at 40 across +0.53–0.86 ms.

**Recommendations, best first (fit to Forge, then patent safety)**

1. **Give each nearby room its own probe grid** (the main build).
   - The generator's room boxes (D-068's `REGION_ROOM` volumes) each get a small grid, anchored in the world, with a layer of probes 0.3–0.4 m in from every wall, floor and ceiling.
   - A room's surfaces, and probe-ray hits in it, read only that grid. The cascades keep lighting the outside and the far rooms.
   - This fixes both measured causes by design:
     - the grid does not move with the camera, so there is no step;
     - a probe layer always stands in front of each outer wall, so problem 2 cannot happen.
   - It also moves further from Square Enix's US claim 37 (`lighting-patents.md` §7 proposed that change for this reason).
2. **Blend linearly, not by square roots, in room lookups** (a free A/B test; do it first).
   - The square-root blend loses light wherever the eight probes disagree, and 2 m probes disagree more than 1 m ones.
   - The bounce loop then compounds the loss (§1).
   - Inside a region the masks already stop the leaks the square root was meant to soften. No patent applies.
3. **A wider room-sized cascade whose outer ring keeps only room probes** (a bridge if item 1 waits).
   - Give each cascade its own counts, for example the 1 m cascade 40 wide and the others unchanged.
   - Probes in the new outer ring that are in no room go inactive.
   - This moves the step from about 6.5 m to about 14.5 m from the camera. It does not remove it.

**Optional:** a per-room gain between cascades (§2.4). It treats the symptom, so use it only if item 1 is far off.

**Avoid:**
- choosing which probes to update by rank (distance, visibility or staleness): NVIDIA's US 11,790,594 B2 (§3);
- placing fine probes from the visible surfaces and filling them from coarse data: Imagination's US 10,410,408 branch;
- per-probe sets of region bits: Square Enix's US claim 44;
- a new GI architecture.

**A new patent finding for the owner.** NVIDIA's **US 11,790,594 B2** is active to 2041-10-17.
- Claim 1: rank locations that hold irradiance caches, cast rays from the higher-ranked ones, and render with the result.
- Dependent claims rank by distance to the camera (claim 7) and by frames since the last update (claim 9).
- Forge's cadence (#103) may come near it. Young probes trace every frame; settled ones trace "every `cadence` frames, on its turn". That is a fixed round robin, not a ranking.
- This is a question for counsel. It is not a reason to change anything today. The village and the town run cadence 1, every probe every frame.

---

## 1. Why the 2 m cascade lights a room darker (*derived* from #353)

#353's rows A, B and 3 show where the gap comes from:
- **Most of it is the probes' own bounce.** The 2 m probes are close to right when the 1 m cascade lights their hits (row 3), and too dark when the 2 m one does (row B).
- **So each 2 m lookup returns a little less light in rooms,** and the infinite-bounce loop compounds the loss.
  - Example: a room whose surfaces return a share ρ = 0.6 of their light (assumed), with each lookup losing 10 % (k = 0.9).
  - The probes then settle at (1 − ρ)/(1 − ρk) = 0.87 of the right value. The surface's own lookup takes another 0.9, giving 0.78 in all.
  - The measured gap, 111.9 against 124.6 sRGB, is about 0.79 in linear light, so **a loss of about 10 % per lookup is enough to explain it.**

Three per-lookup losses in rooms grow with the spacing:
1. **Missing near-wall probes (the masks).** In the 2 m cascade, the probe layer nearest an outer wall stands in the wall, a recess or outside. So the wall reads probes up to 2 m back (#353's game measurements).
   - Ward's irradiance-caching rule says a record is valid only within a radius set by its distance to the surfaces around it (Ward 1988, *search-recorded*).
   - A probe 2 m back cannot represent the steep rise in light just in front of a wall that faces a sunlit floor.
2. **The square-root blend's Jensen darkening** (§2.2). It grows with the spread among the eight probes, so it is larger at 2 m.
3. **Coarser visibility.** #353 measured that turning Chebyshev off makes things darker, so it is not the cause, but it is part of how the 2 m cascade behaves.

The temporal blend is not a cause. Forge blends each update linearly (`lerp(measured, old, h)`). RTXGI blends in a gamma-5 encoding, which biases noisy probes low, and Forge should stay linear (*read*: RTXGI, and the 2021 paper's "exponent of 5.0f").

---

## 2. Techniques

### 2.1 Room probe grids (per-room probe sets)

**How it works.** Each room is lit by a probe set chosen for it, not by a camera-centred grid filtered with masks.

**Prior art:**
- **The Source engine.** VRAD stores a few "ambient cubes" in each visleaf (a convex cell of the map), and what stands in the leaf reads them (Valve wiki, *search-recorded*). Room-local probes have shipped since 2004.
- **Blender EEVEE** advises "When lighting indoor environments, try to align grids with the room shape". It adds that bad samples can be fixed with a smaller grid nearby, and that large scenes "may require using many volumes with different level of details" (*read*).
- **DDGI 2021** adds "high-resolution volumes … to efficiently cover hero assets". Volumes are blended from the most dense to the least, each falling off linearly over its last cell, until the weights reach 1 (*read*).
- **RTXGI's UE4 plugin:** where volumes overlap, "the densest volume will be selected", with a priority and a blend distance (*read*).
- **Unity's APV:** volumes with their own spacing, and rendering layers that split interiors from exteriors (*read*).

**What it changes in Forge:**
- **The grids:**
  - one per `REGION_ROOM` volume (an oriented box with a floor and a ceiling);
  - n = 1 + ⌈(L − 2·inset)/1.5 m⌉ probes per horizontal axis, with an inset of 0.3–0.4 m (beyond relocation's quarter-cell minimum);
  - two or three layers vertically per storey; a stair hall through every storey gets one tall grid;
  - a room of several volumes gets a grid per volume, the seam ramped as a doorway.
- **On the GPU:**
  - a `RoomGrid` record: the box's frame, its counts, its first slot in a room-probe pool, and its region;
  - the pool takes extra tile rows in the existing atlases (512 B of irradiance and 1 KB of distance per probe, from `probes.rs`), and the rays, state and blend passes run over it unchanged;
  - relocation, for furniture, clamped inside the box; classification still turns off probes inside furniture.
- **The lookup:**
  - `region_at` already gives the region; add `grid_of_region[region]`;
  - where a grid exists, a `room_grid_irradiance`: local coordinates, trilinear weights, facing, Chebyshev and the crush, a linear blend. It has no masks, no coverage test, no cascade loop and no far-room fallback;
  - doorways ramp between the two rooms' grids with the existing `mask.ramp`;
  - a window's inner reveal clamps to the room's box;
  - probe-ray hits in such a room use the same lookup, so the bounce loop stays in the room.
- **The cascades:** a cascade probe that stands in a room with a grid goes inactive. That removes the finest cascade's in-room probes and the 0.14–0.23 ms of `PROBE_LEAST_FRONT` lookups they cost in the village (a comment in `probes.slang`).
- **The range:**
  - grids cover the rooms of houses whose insides are real (`probe_regions` already limits itself to "near" houses), within a radius R;
  - start R at about the largest room plus the room-sized cascade's reach, with hysteresis: in at R, out at R + 10 m;
  - each room crossfades from the cascade lookup to its grid over about 0.5 s after its eight young updates. One float per room is enough.
- **The checks:**
  - #353's drift test: the far wall flat to within the noise under `--probe-drift`;
  - the village's "room" view, where rows A and B should become identical, and the map check;
  - the F1 overlay and `tools/amd-check.sh` (the design needs no subgroups).

**Cost (*derived*, to be measured):**
- **Probe counts:** a 4 × 4 × 2.7 m room holds about 32 probes and a six-room house about 200; the 20 × 8 m hall about 250. The camera's house and a few neighbours make 1,000–2,000 probes, against 6,912 slots in one cascade.
- **Scaling:** #353's widening measured 0.026–0.028 ms per 1,000 extra probe slots, inactive ones included. If an active probe costs two to five times an average slot, 1,000–2,000 room probes cost about 0.05–0.3 ms, less what the in-room cascade probes no longer cost.
- **The lookup** likely costs less than today's masked loop over one or two cascades.
- **Memory:** about 1.5 KB per probe, so about 3 MB for 2,000.
- **Every probe updates every frame** with the same ray count, so no ranking is needed (§3).

**Risks:**
- rooms of odd shape need more than one box;
- open-plan spaces without room volumes stay on the cascades;
- the boundary at R moves the remaining step outside, where interiors show only through windows.

**Patents:**
- **Square Enix:** choosing the probe set by the surface's room up front is the change `lighting-patents.md` §7 names as moving away from US and CA claim 37. Keep one region number per grid, never bitsets.
- **Imagination** (the US 10,008,032 B2 family): the grids are ray-traced probes, the same reading as today's probes, which the owner lives with (D-068).
- **NVIDIA's US 11,790,594 B2:** don't schedule rooms by rank (§3).
- **Room-aligned grids:** no patent on them was found. A Tencent patent places probes around voxelised shadow volumes (§3), a different method.

### 2.2 Square-root against linear blending (Jensen)

**How it works:**
- Forge blends the eight probes as (Σ wᵢ√eᵢ / Σ wᵢ)², as RTXGI does: "Decode the tone curve, but leave a gamma = 2 curve to approximate sRGB blending", then "Go back to linear irradiance" (*read*, `Irradiance.hlsl`).
- Flax ships the same default (`DDGI_SRGB_BLENDING 2`, "simple square/sqrt"). It also offers a custom gamma (0.75) and a linear blend (*read*).
- The DDGI 2021 paper's own perceptual step is the exponent-5 encoding of the temporal blend (*read*).

**The bias (*derived*):**
- By Cauchy–Schwarz, (Σw√e)² ≤ Σwe, with equality only when the eight values agree. The relative loss is Var_w(√e) / E_w(e).
- Example: two probes at 4 and six at 1, equal weights. The linear blend gives 1.75 and the square-root blend 1.5625, a loss of 10.7 %, about 6 levels near 120 sRGB.
- At half the spacing the √e differences roughly halve and the loss roughly quarters, to about 3 %.
- So the blend alone can supply a large part of §1's 10 % per lookup, before the loop compounds it. Measured in the village room, it made up about 1.5 levels of the 2 m cascade's gap (see the header).

**What it changes:**
- in `cascade_irradiance`, when `room_mask` is set, blend e (and e_d) linearly; optionally try a power p between 0.5 and 1 (Flax's custom gamma);
- keep the crush below 0.2, which still damps faint leaks from furniture and columns;
- apply it to surfaces and to probe-ray hits alike;
- check rooms' corners and furniture for leaks inside the room.

**Cost:** none. **Patents:** none; it removes a step rather than adding one.

### 2.3 Per-cascade counts, with a rooms-only outer ring

**How it works.** Cascades need not share counts.
- DDGI 2021 cascades volumes of "progressively decreasing grid resolutions … out from the camera". Its camera-locked fix is to tighten the transition by a cell and centre it on the camera (*read*).
- Godot's SDFGI exposes "Cascade 0 Distance", and says a larger one makes the nearest transition less noticeable (*search-recorded*).

**What it changes:** each `ProbeCascade` carries its own counts and first atlas row (built in d22d3779). In the finest cascade's new outer ring, the state pass would mark probes outside every room region inactive. The outdoor 2 m light is fine; the step is an indoor problem.

**Reach** (*derived*: reach = (N − 3)/2 × spacing, which gives #353's 10.5 m at N = 24, and the game's 15.75 m at 1.5 m spacing):

| Finest N | Reach | Fully fine within (reach − `ROOM_BLEND`'s 4 m) |
|---|---|---|
| 24 | 10.5 m | 6.5 m |
| 40 | 18.5 m | 14.5 m |
| 52 | 24.5 m | 20.5 m (the 20 m hall from one end) |

**Cost:** measured (d22d3779, the village, serially), +0.21–0.32 ms at 32 across and +0.53–0.86 ms at 40, before a rooms-only ring would remove the ring's outdoor share. Updating the ring less often would cut it further, but a ring updated less often reads on NVIDIA's claims 1 and 7 (§3): ask first, or keep the updates uniform.

**Fit:** it moves the step farther and shrinks it on screen, but keeps it. 52 across would cost more than room grids.

**Patents:** cascaded volumes are in DDGI 2021, and NVIDIA's DDGI application is abandoned (*prior note*). Imagination as today.

### 2.4 Calibrating a coarse level against a finer one

What others do at the boundaries between levels:
- **Unity's APV:** at bake, "values from boundary probes in the smaller brick are replaced with interpolated values from the larger brick", so subdivision levels meet without a seam (*read*, Unity's issue tracker). A "Sampling Noise" option jitters the sample position to hide seams (*search-recorded*).
- **DDGI 2021:** a linear fade over the last cell, tightened and centred on the camera (*read*).
- **Flax:** a dither over 2 probe spacings by default, a smooth blend as an option (*read*).
- **Godot's SDFGI:** a probe that scrolls in at a cascade's edge starts from the parent cascade's trilinear value, a warm start (*read*, `sdfgi_integrate.glsl`).
- **Forge:** a fade over 4 spacings (`ROOM_BLEND`, ebbd2075).

None of these corrects the coarse level's steady-state bias; they hide the seam. The nearest prior art for a ratio correction is Black Ops II, which divides a reflection probe by the average irradiance where it was captured and scales it by the irradiance at the pixel (Lazarov 2013, *search-recorded*).

**A per-room gain for Forge:**
- per region and coarse cascade c, g_c(R) = the mean luminance of the finer cascade's lookup over the overlap, divided by cascade c's, measured at that cascade's probe positions in R;
- held with a slow exponential moving average while R is partly inside the finer cascade, and applied to cascade c's surface lookups for R, never inside the probes' own bounce, which would feed it back;
- in the hall the camera stands inside the room, so the gain is always measured there.

**Fit:** it only scales the mean, and a wall with a window can stay wrong locally. When the room's lighting changes, the gain lags until the fine cascade sees it again. #353's game measurements show the gap on one wall only (the hall's other walls and its ceiling agree within 2 levels), so a room's gain would brighten those wrongly. A stopgap at best.

**Cost:** a reduction per room-sized region and a float per region and cascade. **Patents:** none found; its prior art is from 2013.

### 2.5 Companion probes by outer walls (the coarse-only version of 2.1)

#353's region-aware relocation moved probes into rooms and gave +4 in one room and −6 in the next. One plausible reason is that moving a probe changes what it lit before.
- **The additive variant:** for a coarse probe in `REGION_SOLID`, a recess or outside, with a room within half a cell, add a companion probe in that room 0.3 m from the wall, tagged with the room's single region number. The original stays as it is.
- In a masked lookup, a corner probe of the wrong region is replaced by its companion of the right region, at the same trilinear weight.

**Fit:** it fixes cause 1 only, in the coarse cascades only, with less reach than 2.1. **Cost:** the cells along the perimeter only. **Patents:** one region number per companion is equality, not a mask (Square Enix's claim 44 needs bits and an AND).

### 2.6 Sparse and adaptive fine probes

Who does it:
- **Unity's APV** fills volumes with 4 × 4 × 4 bricks whose spacing follows the geometry (1, 3, 9 or 27 m) and admits "Light Probes might not follow walls" (*read*).
- **Lumen** keeps its world radiance cache in clipmaps, sparser with distance (*search-recorded*; the 2021 slides, a 122 MB PPTX, were not fetched).
- **Papers:** non-uniform placement along scene skeletons (Wang et al. 2019, *search-recorded*), and "an order of magnitude increase in probe count" by placing probes where the radiosity changes (Datta et al. 2023, *read* the abstract).

**Fit:** Forge's classification already skips the rays of empty-air probes. The cost lies in the active probes near geometry, which sparse storage does not remove; it mainly saves memory.

**Patents:** Imagination's US 10,410,408, 11,734,879 and 12,243,156 cover "low-level" probes whose positions come from the visible surfaces, filled from "high-level" probes (*prior note*). If sparse fine probes are ever built, place them from the scene's room volumes, not from the visible surfaces, and don't seed them from the coarse cascade's data.

### 2.7 Irradiance gradients

**How it works:** Ward and Heckbert (1992) estimate translational and rotational gradients from stratified hemisphere samples and extrapolate to first order (*search-recorded*). Annen et al. (2004) do the same for spherical-harmonic coefficients: "one is often sufficient" sample (*search-recorded*). A coarse probe could extrapolate towards a wall it stands 2 m from.

**Fit:** low for now:
- three times the irradiance data;
- gradients from 128 randomly rotated rays per frame are noisy;
- the gradient is wrong past occluders.

**Patents:** the 1992 and 2004 work is old enough that any patents on it would have expired. Disney's US 8,704,831 B2 (to 2031-11-19) claims "gradient rigs" only as dependents of a per-light choice between analytic and rig evaluation (*read*), which Forge does not make.

### 2.8 Other architectures (not now)

- **Probes on the surfaces:** Lumen's screen probes with a world cache for distant light (*search-recorded*). GI-1.0 says screen probes "do not suffer from light and occlusion leaks" (*prior note*). Both avoid masks by construction, but they are a new gather with its own denoiser.
- **Surfels:** EA's GIBS (*read*, the overview pages; the slides were not parsed).
- **Radiance cascades:** 3D work stores probes in a sparse hashmap and weights cascades by hit distance (Freeman, Sannikov, Pitt-Francis, July 2026, *read* the abstract). Levels merge by interval, so no level switch shows, but it is a new system.
- **Remedy** (Quantum Break's multi-scale volumes) **and The Division** (indoor bleeding): their methods were not found. The notes PDF was over 10 MB, and the GDC Vault talk is behind a login.

---

## 3. Patents

**Scope and limits:**
- The search used web search records of Google Patents and the patent pages themselves.
- Google Patents' search pages render nothing for WebFetch, and Justia's assignee pages returned 403, so assignee-wide lists for Unity, EA, Epic, Activision and Ubisoft could not be pulled.
- Chinese filings were searched only lightly.
- "None found" means none found in this search, not a clearance.

| Technique | Patent | Holder | Jurisdiction | Status, expiry (Google's fields) | Bears on | Unpatented route |
|---|---|---|---|---|---|---|
| Ranking locations for ray casting | **US 11,790,594 B2** (US 2021/0049807, WO 2021/034837) | NVIDIA | US, PCT | Active, adjusted expiry 2041-10-17 (*read*) | Claim 1: rank cached locations, cast rays from the higher-ranked ones, render. Claims 7 and 9: by camera distance, by frames since the last update. Claim 16: an outgoing-irradiance cache at the hit. **Forge's cadence (#103)** and any ranked update of rings or rooms come near claim 1. Claim 16 needs an outgoing cache at the hit, which Forge's probes are not | Update every probe every frame with uniform rays; counsel on #103 |
| The same family, a continuation | US 2024/0029338 A1 | NVIDIA | US | Pending (*read*) | Claim 1 denoises a mix of per-pixel samples and cache values; no technique here | — |
| Ray-traced probe updates | US 10,008,032 B2 family (including 10,410,408; 11,734,879; 12,243,156; GB 2571656 B) | Imagination | US, GB | Active to 2036-03/08 (*prior note*) | Every option here is ray-traced probes: no change from D-068. The low-level-probe branch reaches sparse probes placed from visible surfaces and filled from high-level probes | Grids from the plan's room volumes, never seeded from coarse data |
| Probe culling by zones | US 10,137,372 B2; EP 3306569 B1; JP; CA | Square Enix | US, EP, JP, CA | Active to 2037-02-01 in the US (*prior note*) | Claim 37 (zones, a grid of regions) and claim 44 (a mask, a bitwise AND). Room grids choose the set by room up front, which moves away from them | One region number per grid or companion, compared for equality |
| DDGI (moments, relocation, classification, volumes) | US 2021/0012562 A1 | NVIDIA | US | Abandoned 2023-08-23 (*prior note*) | Cascades, relocation, classification | Free |
| Probes placed around voxelised shadow volumes | US 11,854,143 B2 | Tencent | US (CN priority) | Active, adjusted to 2041-06-19 (*read*; claims not read in full) | Automatic placement. Room grids use no shadow points or voxels | Grids from room boxes |
| Gradient rigs | US 8,704,831 B2 | Disney | US | Active, adjusted to 2031-11-19 (*read*) | Claim 1 needs a per-light choice between analytic and rig evaluation; gradients are dependent claims | Ward–Heckbert or Annen gradients, if ever |
| Neural denoising of probe data | US 12,260,527 B2 (a continuation pending) | Unity | US | Granted 2025-03-25 (*search-recorded*) | None of these techniques | — |
| APV bricks, virtual offset, anti-seam, rendering layers | none found | Unity | — | — | Ideas only | — |
| GIBS surfels | none found | EA | — | — | — | — |
| Lumen's radiance cache and screen probes | none found | Epic | — | — | — | — |
| CoD IW's grid; Black Ops II's normalisation | none found | Activision | — | — | §2.4's gain | Prior art from 2013 |
| Probe volumes, normalised irradiance probes | none found | Ubisoft | — | — | — | — |
| Radiance cascades | none found | — | — | — | — | — |
| Per-leaf ambient cubes; irradiance caching (1988) and gradients (1992) | none found; any would have expired | Valve; Ward and others | — | — | Prior art for 2.1 and 2.7 | Free |
| Square-root against linear blending | none | — | — | — | §2.2 | Free |

Checked and irrelevant: Tencent's US 12,293,451 B2 (precomputed radiance transfer under light changes) and Square Enix's US 9,275,477 B2 (specular roughness from sampled paths), both *read*.

---

## 4. Verification notes

- **Read:**
  - the DDGI 2021 paper (ar5iv);
  - RTXGI's `Irradiance.hlsl`, `DDGIVolume.md` and UE4 README;
  - Flax's `DDGI.hlsl`, Godot's `sdfgi_integrate.glsl`, Blender's volume-probe manual;
  - Unity's APV concept, light-leak and artefact pages, and issue 15471;
  - the Lumen technical page and the Advances 2021 index;
  - the abstracts of arXiv 2301.05125 and 2607.20384;
  - EA's GIBS news pages;
  - the Google Patents pages for US 11,790,594 B2, US 2024/0029338 A1, US 11,854,143 B2, US 8,704,831 B2, US 12,293,451 B2 and US 9,275,477 B2.
- **Search-recorded:** the Valve wiki, Lazarov 2013, Ward 1988 and Ward–Heckbert 1992, Annen 2004, Wang et al. 2019, Unity's Sampling Noise, Godot's cascade setting, Lumen's clipmaps, Unity's US 12,260,527.
- **Not found:** how Remedy, The Division, Frostbite and CoD IW keep levels consistent indoors; Unity's run-time handling of mixed brick sizes beyond the bake-time anti-seam.
- **Derived:** §1's loop arithmetic, §2.2's example, every reach and cost except those measured, and ρ = 0.6.
- **One file landed on disk unintended:** WebFetch could not parse the GIBS 2024 PDF and saved it (7 MB) outside the repository. It was never opened, and it was deleted the same night.

## Sources

- **Forge:** #353 https://github.com/Eolan/forge-engine/issues/353 ; `docs/research/probe-leaks-indoors.md`, `docs/research/lighting-patents.md`, D-068 in `docs/DECISIONS.md`
- **Majercik et al.,** "Scaling Probe-Based Real-Time Dynamic GI for Production", JCGT 2021: https://ar5iv.labs.arxiv.org/html/2009.10796
- **RTXGI-DDGI:**
  - https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/rtxgi-sdk/shaders/ddgi/Irradiance.hlsl
  - https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/docs/DDGIVolume.md
  - https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/ue4-plugin/4.27/RTXGI/README.md
- **Flax Engine DDGI:** https://github.com/FlaxEngine/FlaxEngine/blob/master/Source/Shaders/GI/DDGI.hlsl
- **Godot:**
  - https://github.com/godotengine/godot/blob/4.3-stable/servers/rendering/renderer_rd/shaders/environment/sdfgi_integrate.glsl
  - https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/using_sdfgi.html
- **Unity's APV:**
  - https://docs.unity.com/en-us/engine/6000.0/manual/lighting-overview/lighting/probevolumes/concept
  - https://docs.unity3d.com/Manual/urp/probevolumes-troubleshoot-light-leaks.html
  - https://docs.unity3d.com/Manual/urp/probevolumes-troubleshoot-artefacts.html
  - https://issuetracker.unity.com/issues/15471/seams-and-leaks-at-boundaries-between-subdivision-levels-when-using-rendering-layers
  - https://docs.unity3d.com/Manual/urp/probevolumes-options-override-reference.html
- **Blender EEVEE's volume probes:** https://docs.blender.org/manual/en/latest/render/eevee/light_probes/volume.html
- **Valve, ambient cubes:** https://developer.valvesoftware.com/wiki/Ambient_cube
- **Epic, Lumen:**
  - https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine
  - https://advances.realtimerendering.com/s2021/index.html
  - https://www.unrealengine.com/en-US/tech-blog/unreal-engine-5-goes-all-in-on-dynamic-global-illumination-with-lumen
- **AMD, GI-1.0:** https://ar5iv.labs.arxiv.org/html/2310.19855
- **EA, GIBS:**
  - https://www.ea.com/seed/news/siggraph21-global-illumination-surfels
  - https://www.ea.com/technology/news/gibs-lighting-ea-sports-college-football-25
  - https://advances.realtimerendering.com/s2024/content/EA-GIBS2/Apers_Advances-s2024_Shipping-Dynamic-GI.pdf
- **Remedy, Quantum Break:** https://www.advances.realtimerendering.com/s2015/SIGGRAPH_2015_Remedy_Notes.pdf
- **Activision:**
  - https://research.activision.com/publications/archives/precomputed-lighting-in-call-of-dutyinfinite-warfare
  - Lazarov 2013: https://blog.selfshadow.com/publications/s2013-shading-course/lazarov/s2013_pbs_black_ops_2_notes.pdf
- **The Division:** https://gdcvault.com/play/1023273/Global-Illumination-in-Tom-Clancy
- **Probe placement:**
  - Wang et al., I3D 2019: https://cs.mcgill.ca/~kry/pubs/i3d2019/NonUniformProbes-paper.pdf
  - Datta et al. 2023: https://arxiv.org/abs/2301.05125
  - Freeman, Sannikov, Pitt-Francis 2026: https://arxiv.org/abs/2607.20384
- **Irradiance caching and gradients:**
  - Ward–Heckbert 1992: https://www.ri.cmu.edu/publications/irradiance-gradients
  - Ward 1988, via Jarosz: https://cs.dartmouth.edu/~wjarosz/publications/dissertation/chapter3.pdf
  - Annen et al. 2004: https://jankautz.com/publications/gradientSH_RS04.pdf
- **Patents:**
  - https://patents.google.com/patent/US11790594B2/en
  - https://patents.google.com/patent/US20240029338A1/en
  - https://patents.google.com/patent/US10008032B2/en
  - https://patents.google.com/patent/US10137372B2/en
  - https://patents.google.com/patent/US20210012562A1/en
  - https://patents.google.com/patent/US11854143B2/en
  - https://patents.google.com/patent/US8704831B2/en
  - https://patents.google.com/patent/US12260527
  - https://patents.google.com/patent/US12293451B2/en
  - https://patents.google.com/patent/US9275477B2/en
