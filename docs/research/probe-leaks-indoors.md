# Probe GI leaks indoors: thin walls, reveals and doorways

> Written 2026-10-17 (#266), after the village's finest probe cascade moved to 1 m (ef27049).
> Companion to `lighting-gi.md` ("Implementation notes from Forge: DDGI probes"),
> `interior-lighting.md` (#270) and `local-exposure.md` (#275). Sources are marked *read*
> (fetched that day) or *search-recorded* (seen only through a search engine's record). Forge's
> facts come from `shaders/probes.slang`, `probe_update.slang` and `meshlet.slang` at ef27049.
> The two live patents in §4 were checked again on Google Patents the same day (status and
> expiry). Patent facts are Google Patents' records, whose status fields are its own
> assumptions. This is not legal advice.

**The short answer.**
- **Leak 1 is a soft test against a hard contrast.** The Chebyshev test lets a few thousandths of
  a probe's weight through a 0.1–0.15 m partition. A sunlit room is 100–1000 times brighter than
  the room behind it, so those thousandths show.
- **Leak 2 is not a visibility failure.** The sunlit probe outside really does see the window
  head through the opening. It stands in different light, and no visibility statistic can
  reject it.
- **Others add a second test.**
  - Unity ships "rendering layers", so interior surfaces read only interior probes.
  - Call of Duty: Infinite Warfare bakes per-probe depth toward its neighbours.
  - Godot stores eight occlusion values per voxel.
  - Square Enix patented per-probe zones.
  - Unreal's volumetric lightmaps offer only smaller cells or thicker walls.
- **First change for Forge: regions from the generator.**
  - Tag each probe and each surface with a room, the outside, or an opening's recess.
  - A probe gets no weight on another region's surface. Doorways and stair holes are portals,
    ramped across their depth.
  - It fixes leaks 1 and 2 by construction, with no rays, at an estimated 0.02–0.06 ms.
  - Two near-free fixes go with it: store back-face hits at a fifth of their distance (RTXGI),
    and bias with the interpolated normal.
- **Patents.**
  - The nearest live patent to the region test is Square Enix's zone culling: US 10,137,372 B2
    (to 2037) and EP 3306569 B1. Forge's design differs in how zones are made; ask the owner.
  - Imagination's US 10,008,032 B2 family (to 2036) claims ray-traced probe updates in general.
    That bears on Forge's probes as they are (D-036), not on these fixes.

---

## 1. Why each leak passes today

Forge's lookup follows RTXGI's `DDGIGetVolumeIrradiance` closely:
- the back-face weight ((dot + 1)/2)² + 0.2;
- a Chebyshev weight, cubed and floored at 0.05;
- the crush under 0.2 and the trilinear weight;
- the square-root blend, which is RTXGI's decode "to approximate sRGB blending".

It differs in two places:
- **Back-face hits** keep their full distance; RTXGI stores `-hitT * 0.2f`.
- **The normal.** Where a material has a normal map, the lookup gets the bent normal
  (`sun_light`), and it also sets the bias and the back-face weight.

**Leak 1, a partition.** The arithmetic is derived; values marked as assumed are guesses.
- Relocation keeps a probe at least 0.25 m from the partition.
- The shaded point lies the wall's thickness plus 4.5 cm of normal bias beyond the face the
  probe sees: a gap of 0.15–0.3 m.
- A 14 × 14 distance texel spans about 14°. At grazing incidence the wall's depth varies across
  it by ±0.1–0.2 m, and the cos⁵⁰ lobe and the bilinear fetch widen that.
- With σ = 0.2 m and a 0.2 m gap (assumed), Chebyshev gives 0.5, cubed 0.125. After the
  back-face weight (≤ 0.45) and the crush, the probe keeps about 5 × 10⁻³ of a front probe's
  weight.
- Against a room 400 times brighter (assumed), that adds about 20 % through the square-root
  blend: a soft tinted blotch.

The 2022 DDGI paper ends on this case: walls that are "flat planes that receive strong
sunlight", and "further work is needed in making DDGI more resilient to thin geometry" (*read*).

**Leak 2, the reveals.**
- **The cap.** A probe outside sees the window head through the opening, with Chebyshev near 1
  and a high back-face weight. It lights the head with street light.
- **Torn edges.** Its distance texels straddle the opening's edge.
- **A probe in the recess.** One relocated there stands half in the sun.

**Leak 3, the doorway at 2 m.** A nearly binary weight draws hard edges: the cube and the crush
switch a probe off within centimetres where a jamb crosses its texels. McGuire's GDC 2017 slides
say visibility tests on probes create "hard shadow line errors" (*search-recorded*).

## 2. What engines and papers do

### DDGI and RTXGI
- **Majercik et al. 2019.** McGuire's page lists the limitation "Cannot prevent leaks from
  zero-thickness/single-sided walls". The GTC 2019 deck adds "Self-shadow bias must be tuned to
  geometry thickness" and "No patents on the algorithm" (*read*).
- **Majercik et al. 2021** (*read*).
  - Relocation, probe states, and the bias (0.2n + 0.8ω)·0.75·D·B with B = 0.3.
  - Back-face hits record irradiance 0 and depth "shortened by 80%". Not 0: that would push the
    Chebyshev weight to 0 and skew probes that see a few back faces.
  - Leaks remain at "complex geometry with acute corners".
- **RTXGI SDK 1.3 defaults** (*read*).
  - `probeDistanceExponent` 50 ("risks causing banding").
  - `probeIrradianceEncodingGamma` 5, so temporal blending is perceptual.
  - `probeRandomRayBackfaceThreshold` 0.1: a probe past it is not blended that update.
  - `probeFixedRayBackfaceThreshold` 0.25, and relocation of at most 45 % of a cell.
  - Probe Variability (1.3.5) tracks convergence, not leaks.
  - The docs: "If walls are too thin relative to the volume's probe density, you will observe
    light leaking."
- **Majercik et al. 2022.** Bias "along the view path", "guaranteed to not penetrate surfaces"
  (*read*).

### Visibility computed another way
- **Light field probes** (McGuire et al., I3D 2017). Per-texel visibility "similar to a
  G-buffer", precomputed for static scenes (*search-recorded*).
- **SDFDDGI** (Hu et al. 2020, *read*).
  - An SDF shadow trace from the point to each probe replaces the moments.
  - Checkerboarding and de-duplication cut "8 tests per pixel to 0.25 tests per pixel or lower".
  - Total GI: 1.67 ms against RTXGI's 3.98 ms (RTX 2080 Ti, 1080p).
  - It needs scenes hand-simplified into SDF primitives.
- **Godot SDFGI.**
  - Eight occlusion values per voxel, one per surrounding probe, 4 bits each.
  - They are made by propagation through the voxels' solid-face masks
    (`sdfgi_preprocess.glsl`, *read*).
  - Its known cost is darkened corners. HDDAGI kept it: "I experimented with other systems (the
    one in DDGI) but everything has worse tradeoffs" (PR #86007, *read*).
- **Call of Duty: Infinite Warfare** (Iwanicki & Sloan 2017, *read*). Without visibility "the
  lighting will bleed through walls". Each probe of the tetrahedral grid stores a triangular
  depth map toward each tet's opposite face: 15 values of 8 bits. The grid holds about 100k
  probes in 60 MB, with lookups of 0.2–0.4 ms on async compute. All baked.
- **Square Enix** (filed by Eidos Interactive). Per-probe occlusion planes cut space into zones,
  and a precomputed mask says which zones each probe sees (§4).
- **Others.**
  - Quantum Break multiplies the trilinear weight by a normal term (*search-recorded*).
  - Enlighten's precomputed probe visibility "can sometimes cause probe lighting to change
    suddenly" (*read*).

### Regions and layers
- **Unity's Adaptive Probe Volumes** (*read*).
  - Virtual Offset and Dilation run at bake time.
  - Leak Reduction: "Performance" warps the sampling coordinates away from invalid probes.
    "Quality" samples 1–3 times and is the default since 6000.0.3f1.
  - Rendering Layers (6000.1): up to four masks; renderers "only sample lighting data from
    probes that have a matching Rendering Layer Mask". The documented case is an Interior and an
    Exterior mask. A renderer matching no mask reads all valid probes.
  - Adjustment volumes fix "most light leak issues, but often not all".
- **The Division** (GDC 2016). The abstract promises "how to handle probe bleeding in indoor
  areas"; the method was not found (*search-recorded*).

### Probes only where rays do not reach
- **Unreal's volumetric lightmaps.** "The only current solutions to leaking are to decrease" the
  cell size, or thicken the wall (*read*).
- **Lumen.** Gathers from screen probes on the G-buffer (*search-recorded*). It wants walls "no
  thinner than 10 centimeters" (*read*).
- **AMD's GI-1.0.** Screen probes "do not suffer from light and occlusion leaks as they are
  always placed precisely on the geometry" (*read*).
- **EA's GIBS** (Apers 2024, *read*).
  - Surfels light most surfaces. A DDGI-like probe clipmap does the rest: 14 × 14 variance depth
    and 6 × 6 irradiance, borders included, at most 100k probes.
  - Its variance depth stopped dark probes leaking through stadium stands.
  - Its surfels take the vertex-interpolated normal, not a reconstructed one.

## 3. Side by side

| Technique | Leak 1 | Leak 2 | Leak 3 | Cost (estimate) | Fails when | Dynamic and procedural |
|---|---|---|---|---|---|---|
| Regions from the plan | Fixed | Fixed | Likely; portals ramped | 0.02–0.06 ms | A region with no probe (fallback) | Yes |
| Back-face distance × 0.2 | Partly (gaps, probes by walls) | No | No | 0 | Walls without gaps | Yes |
| Interpolated normal for the bias | A little | A little | No | 0 | — | Yes |
| Chebyshev tail cut | Mostly | No | Harder edges | ~0 | Dark corners | Yes |
| Per-pixel visibility rays | Fixed | No: the sight line is real | Worse | 0.3–1 ms | Hard lines | Over budget |
| Per-voxel corner occlusion (Godot) | Fixed | If glass is solid | Soft | Voxels or rays per scroll | Dark corners | Needs voxels |
| Per-probe zone masks (Square Enix) | Fixed | Fixed | Hard | Bake | Baked; live patent | No |
| 0.5 m spacing | Partly | No | Partly | ~8× the probes | 0.1 m walls still thinner | Too costly |

## 4. Patents

The search covered Google Patents records, found by web search, for NVIDIA, Unity, Activision,
Square Enix, Imagination and Tencent. It used probe visibility, leak and zone terms, plus
variance shadow maps. **This is not a professional search.**

| Technique | Patent | Holder | Jurisdiction | Status, expiry | Bearing |
|---|---|---|---|---|---|
| DDGI moments, relocation, states, back-face shortening | US 2021/0012562 A1 | NVIDIA | US | **Abandoned** 2023-08-23, after an appeal | No live patent from it. The GTC 2019 deck: "No patents on the algorithm". |
| Ray-traced probe updates | US 10,008,032 B2; GB 2571656 B; continuations US 10,410,408, 10,672,183, 11,127,198, 11,734,879 and 12,243,156 B2; US 2025/0157141 A1 pending | Imagination Technologies | US, GB | Active; to 2036-08-26 (GB 2036-03-03); rechecked 2026-10-17 | Claim 1: per frame, find the visible surfaces; per probe, trace rays and update a directional lighting representation; derive the surfaces' lighting from the probes; shade. Dependent claim 10: probes with "line of sight" to the surface. The independent claims describe what Forge's probes (D-036) already do, not the region test; a professional opinion would settle it. The continuations were not read in full. |
| Probe culling by per-probe zones | US 10,137,372 B2; EP 3306569 B1; JP 6970508 B2; CA 2944435 C | Square Enix | US, EP, JP, CA | Active; US to 2037-02-01 (fees paid 2026-05-20), EP to 2036-12-14; rechecked 2026-10-17 | Claim 1: zones "defined by intersections of occlusion planes predefined for that probe", and only probes that see the point's zone are combined. Later claims add a per-probe mask and a bitwise AND. **Nearest to the region test.** Forge would use no per-probe planes and no per-probe masks: one region number per probe and per surface, from the plan, compared for equality. Ask the owner. |
| Unity's rendering layers | None found | — | — | — | Shipped 2025. |
| Chebyshev tail cut; variance shadow maps | None found | — | — | — | Free, as far as found. |
| SDF visibility (Hu et al.) | None in English; Chinese filings not searched | — | — | — | Not recommended. |

**Unpatented route, as far as found:**
- back-face shortening (the NVIDIA application is abandoned);
- the interpolated normal;
- the tail cut;
- region numbers compared for equality. Square Enix's claims are close enough that the owner
  should decide.

## 5. Recommendation for Forge

### First: regions from the generator (leaks 1 and 2)

**Data.** Each house sends its rooms and its openings, plus a lot-scale grid that finds the
house:
- **Rooms:** polygons with floor and ceiling heights, one region number each.
- **Openings:** doors, windows and stair holes. Each is a box through the wall or slab, with the
  two regions it joins and a glazed flag.

**`region_at(p)`** returns the containing room, else an opening's recess, else "solid" (inside a
house but in no room or opening: its walls and slabs), else "outside".

**Probes.**
- `probe_state_main` stores `region_at` of the relocated position at each young update and keeps
  it once settled; wake redoes it.
- Probes in "solid" or in any recess get no region. They light nothing except through the
  fallback, so a probe pushed into a reveal no longer lights the window head.

**Surfaces.**
- The resolve takes `region_at(p + 2 cm · n)`, with the interpolated normal, so a wall face
  belongs to the room it faces.
- A per-meshlet region number would cost one load instead, if the kit splits meshlets at room
  boundaries.

**Weights.**
- A probe weighs 1 in its own region and 0 elsewhere.
- On surfaces inside an open door, a stair hole or an unglazed window, each side's probes get a
  ramp across the opening's depth: 1 at their own face, 0 at the far one. The jambs then shade
  continuously into both rooms.
- At a glazed window, the inner reveal belongs to the room and the outer reveal to the street.

**Fallback.** If none of the eight probes is in the region, the next coarser cascade is tried.
If no cascade has one, the unmasked lookup runs, as Unity's does. Count the fallbacks in the F1
overlay.

**With it:**
- back-face hits stored as −0.2·t;
- the interpolated normal for the bias and the back-face weight. The bent normal stays for the
  irradiance direction, since #70 saw half of #68's sparkles return without it.

**Cost (estimates until F1 measures).**
- Per probe: one more 4-byte load, or the region packed into `probe_data`.
- Per pixel: one grid load and 4–12 room tests.
- No pass and no rays: +0.02–0.06 ms at 1600 × 900.
- Memory: 166 KB for six cascades, plus a few kB per house.
- No subgroups and no groupshared memory, so it runs as is on AMD.

**Checks.**
- **Unit tests** of `region_at` on the kit's walls, reveals, jambs and stair holes. Every room
  must hold at least one probe at 1 m.
- **Views:** the map check, plus a partition with the sun in the next room, a window head from
  inside, and the doorway at 2 m.
- **Measures:**
  - the patch's mean log luminance with masks off and on, and ꟻLIP between them;
  - a still camera under `--dlaa --fixed-step --capture-every 10`, within 0.05 %;
  - the "way in" walk, watched for steps at portals.
- **Debug views:** regions in false colour, and the probes coloured by region.
- **Timing and AMD:** F1, best of three, and `tools/amd-check.sh`.

### Next, if leaks remain inside a region (furniture, columns)
1. **A Chebyshev tail cut,** `linstep(a, 1, c)` before the cube, starting at a = 0.2–0.3.
   Lauritzen's light-bleeding reduction costs "somewhat darkening the penumbra regions", so
   watch room corners.
2. **RTXGI's 10 % back-face threshold** for blending.
3. **A higher distance exponent,** only if measured.

### Avoid
- **Per-pixel visibility rays to the probes.**
  - An estimated 0.3–1 ms.
  - Binary edges are leak 3's artefact.
  - They cannot fix leak 2, whose sight line is real.
  - Imagination's family claims selection by line of sight.
- **Per-probe occlusion planes with zone masks:** Square Enix's claims.
- **A larger bias.** It moves the leak, detaches contact light, and "must be tuned to geometry
  thickness".
- **0.5 m probes:** eight times the probes for a quarter of the reach.
- **Walls thickened toward the spacing** (Unity's advice): that changes the architecture, not the
  light.
- **Light field probes or a screen-probe gather:** a new GI system, with a bake or a denoiser.

## Verification notes

- **Read on 2026-10-17:**
  - the RTXGI sources, docs and release notes;
  - the 2021 and 2022 papers, McGuire's DDGI page, and SDFDDGI and GI-1.0 on ar5iv;
  - the GTC 2019, CoD IW and GIBS 2024 decks, as PDF text;
  - the Unity, Unreal, Godot and Enlighten pages, Godot's shader and PR #86007, and GPU Gems 3
    ch. 8;
  - the Google Patents pages cited (US 10,137,372 B2 and US 10,008,032 B2 fetched again by the
    session that filed this note).
- **Search-recorded:** the GDC 2017 light-field slides (too large to fetch), Quantum Break,
  Lumen's screen probes, and The Division.
- **Not found:** how Control and Frostbite's 2018 baker handle leaks. Treyarch's 2016 PDF refused
  the fetch.
- **Derived:** the leak 1 arithmetic, and every cost.

## Sources

- **Majercik, Guertin, Nowrouzezahrai, McGuire. "Dynamic Diffuse Global Illumination with
  Ray-Traced Irradiance Fields." JCGT 8(2), 2019.** [paper]
  https://www.jcgt.org/published/0008/02/01/ ; https://morgan3d.github.io/articles/2019-04-01-ddgi/
- **McGuire. "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields." GTC 2019
  deck (S9900), April 2019.** [talk]
  https://developer.download.nvidia.com/video/gputechconf/gtc/2019/presentation/s9900-irradiance-fields-rtx-diffuse-global-illumination-for-local-and-cloud-graphics.pdf
- **Majercik, Marrs, Spjut, McGuire. "Scaling Probe-Based Real-Time Dynamic Global Illumination
  for Production." JCGT 10(2), 2021.** [paper] https://ar5iv.arxiv.org/html/2009.10796
- **Majercik, Müller, Keller, Nowrouzezahrai, McGuire. "Dynamic Diffuse Global Illumination
  Resampling." CGF 41, 2022.** [paper] https://arxiv.org/html/2108.05263v1
- **NVIDIA. RTXGI-DDGI SDK.** [code] https://github.com/NVIDIAGameWorks/RTXGI-DDGI
- **McGuire, Mara, Nowrouzezahrai, Luebke. "Real-Time Global Illumination using Precomputed
  Light Field Probes." I3D 2017.** [paper]
  https://research.nvidia.com/publication/2017-02_real-time-global-illumination-using-precomputed-light-field-probes
- **Hu et al. "Signed Distance Fields Dynamic Diffuse Global Illumination." arXiv 2007.14394.**
  [paper] https://ar5iv.labs.arxiv.org/html/2007.14394
- **Unity. "New ways of applying global illumination to your worlds in Unity 6."** [web]
  https://unity.com/blog/engine-platform/new-ways-of-applying-global-illumination-in-unity-6
- **Unity. Adaptive Probe Volumes manual: options, light leaks, rendering layers.** [docs]
  https://docs.unity3d.com/Manual/urp/probevolumes-options-override-reference.html ;
  https://docs.unity3d.com/Manual/urp/probevolumes-troubleshoot-light-leaks.html ;
  https://docs.unity3d.com/Manual/urp/features/rendering-layer-masks-apv-prevent-light-leaks.html
- **Epic Games. "Volumetric Lightmaps"; "Lumen Technical Details."** [docs]
  https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-lightmaps-in-unreal-engine ;
  https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine
- **Godot. SDFGI manual; `sdfgi_preprocess.glsl`; PR #86007.** [docs] [code]
  https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/using_sdfgi.html ;
  https://github.com/godotengine/godot/blob/master/servers/rendering/renderer_rd/shaders/environment/sdfgi_preprocess.glsl ;
  https://github.com/godotengine/godot/pull/86007
- **Iwanicki, Sloan. "Precomputed Lighting in Call of Duty: Infinite Warfare." SIGGRAPH 2017.**
  [talk] https://research.activision.com/publications/archives/precomputed-lighting-in-call-of-dutyinfinite-warfare
- **Apers. "Shipping Dynamic GI." SIGGRAPH 2024 Advances.** [talk]
  https://advances.realtimerendering.com/s2024/content/EA-GIBS2/Apers_Advances-s2024_Shipping-Dynamic-GI.pdf
- **Boissé et al. "GI-1.0." AMD, 2023.** [paper] https://ar5iv.labs.arxiv.org/html/2310.19855
- **Silvennoinen, Timonen. "Multi-Scale Global Illumination in Quantum Break." SIGGRAPH 2015.**
  [talk] https://advances.realtimerendering.com/s2015/
- **Stefanov. "Global Illumination in Tom Clancy's The Division." GDC 2016.** [talk]
  https://gdcvault.com/play/1023273/Global-Illumination-in-Tom-Clancy
- **Silicon Studio. Enlighten, "Per mesh probe lighting".** [docs]
  https://documentation.enlighten.siliconstudio.co.jp/wiki/spaces/SDK402/pages/2485158048
- **Lauritzen. "Summed-Area Variance Shadow Maps." GPU Gems 3, ch. 8.** [book]
  https://developer.nvidia.com/gpugems/GPUGems3/gpugems3_ch08.html
- **NVIDIA. "Probe-based dynamic global illumination." US 2021/0012562 A1 (abandoned).**
  [patent] https://patents.google.com/patent/US20210012562A1/en
- **Imagination Technologies. US 10,008,032 B2; GB 2571656 B; US 12,243,156 B2.** [patent]
  https://patents.google.com/patent/US10008032B2/en ;
  https://patents.google.com/patent/GB2571656B/en ;
  https://patents.google.com/patent/US12243156B2/en
- **Square Enix. "Virtual scene lighting with light probe culling based upon visibility."
  US 10,137,372 B2; EP 3306569 B1.** [patent] https://patents.google.com/patent/US10137372B2/en ;
  https://patents.google.com/patent/EP3306569B1/en
