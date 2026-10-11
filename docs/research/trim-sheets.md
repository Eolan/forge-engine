# Trim sheets, thin-detail aliasing and street-aligned paving

*Research note, 2026-10-11, after the owner's first walk through a generated city: shimmer on the
facades along the streets, paving whose orientation ignores the streets, and the owner's request
to study trim sheets. Sources were read or search-checked by a research agent; the claims that rest
on a single forum post say so.*

**Summary.** Two problems in the procedural European city share one root cause: detail is stored in
the wrong form. Louvre slats (7 cm period), railing bars (1.8 cm every 11 cm) and mullions are real
geometry. Past roughly 30–40 m they fall below the sampling rate, and TAA jitter turns that
undersampling into shimmer. A cluster-DAG cannot fix this, because simplifying disconnected thin
parts does not prefilter them. Texture can be prefiltered: mipmaps average the detail correctly.
Trim sheets are the standard way to keep detail like this in texture form: an atlas of strips,
with UVs snapped to the strips.

The plan below gives each thin kit part a "far form" baked into a trim sheet from the part's own
program, so no downloads are needed. Thin bars in the mid range get a minimum on-screen width. The
ground follows the same idea: each paved region gets its own UV frame (along the street, the kerb
or the facade) and a procedural pattern. Orientation changes are hidden under border strips, which
is how real paving does it.

Patents were found on hashed alpha testing (NVIDIA, active until 2037) and on some
virtual-texturing methods. Unpatented or expired alternatives exist for everything recommended.

## 1. Trim sheets

### 1.1 What they are

A trim sheet is a texture atlas of horizontal strips. Each strip tiles along U only, and each holds
one kind of detail: a moulding profile, a panel edge, a plinth or a cornice. Artists cut a model's
UV islands so that they snap onto the strip that matches them, so many assets share one material
[9][10].

Because a strip spans the full width of the texture, wrapping in U works natively. NVIDIA's 2004
atlas paper explains why wrapping is the usual atlas problem: coordinates outside a sub-texture
read the texture next to it [14]. A strip removes that problem on one axis only, so V must never
wrap.

One Polycount forum thread separates trim sheets (strips of surface) from decal sheets (pieces laid
over another surface). That definition rests on forum posts [10].

### 1.2 How studios use them

- **Insomniac, Sunset Overdrive (GDC 2015, "The Ultimate Trim").** A small team textured an open
  world with one standardised trim layout, a UV mapping script and a custom shader for variation
  [1]. In the layout, strips grow in height, and the normal maps carry 45° bevels on every strip
  edge, so hard edges look bevelled without extra geometry. Materials can be swapped without
  touching UVs [2].
- **Insomniac, Marvel's Spider-Man (GDC 2019).** The Manhattan talks cover city-wide materials built
  with Substance Designer [3] and Houdini-driven generation of the ground, buildings and impostors
  [4][5]. The public abstracts do not mention trim sheets by name, so how trims were used there is
  not confirmed.
- **Naughty Dog, Ubisoft.** No primary source on trim sheets was found from these studios. The
  Uncharted 4 texturing talk is about Substance procedural workflows [6]. The Ubisoft work found is
  on terrain virtual texturing (Far Cry 4) [51], not trims.
- **Embark, The Finals.** The building system is modular in Houdini. Its write-up mentions modules,
  alternate materials and rotating UVs per module, but not trim sheets [7].
- **Unreal/Quixel practice.** No official Epic document on trim sheets was found. There is an
  Epic-hosted community tutorial on trims combined with vertex blending [11].

### 1.3 Combining trims with procedural geometry

Trims suit generated geometry because the UV rule is simple: U equals distance along the edge in
metres, V equals a fixed band. CityEngine's CGA states this choice directly. `setupProjection`
anchors the UVs either to the shape's own scope or to world axes, with texture size given in metres
[13]. Houdini's Labs Automatic Trim Texture node fits a mesh's UV shells onto the strips of a given
trim texture [12].

### 1.4 Texel density and mip-mapping

- **Density.** Texel density means texels per metre of surface. Keep it consistent across all kit
  parts, so that a cornice and the wall under it share the same mip at any distance.
- **Bleeding.** Mips stop leaking between strips if the atlas and its strips are powers of two and
  no strip crosses a power-of-two line. Mips can also be built per strip with clamping at strip
  borders [14].
- **Lowest mips.** Below the mip where a strip is one texel high, strips mix. Clamp the maximum mip
  level for trim materials, or make sure kit parts are never small enough on screen to reach it.
- **Grazing views.** Facades seen along a street are at grazing angles, so 16× anisotropic
  filtering is needed on trim materials.

### 1.5 Trim plus decal

A common pattern puts tiling surfaces and trims underneath, with decals on top for unique detail.
Ultimate Trim's bevel strips are one form of this [2]. id Tech 6 (DOOM 2016) composites large
numbers of decals through its clustered structure [15]. Wronski documents the artefacts of
screen-space deferred decals and how to fix them [16].

### 1.6 Details stored as maps

Louvres and railings can live in a strip as albedo, normal, AO and height, or as coverage (alpha):
- **Parallax occlusion mapping** restores depth at oblique views. Tatarchuk's I3D 2006 paper
  includes an adaptive LOD for the shader cost [17].
- **Coverage in mips.** Castaño's coverage-preserving alpha mips stop alpha-tested detail from
  thinning out with distance [18].
- **Alpha-to-coverage** antialiases alpha edges, but only with MSAA. Golus's article assumes forward
  rendering with MSAA [19].

### 1.7 Authoring tools

- Houdini Labs (Automatic Trim Texture, Trim Texture utility) [12].
- Blender "Trimmer" add-on, GPL-3.0, Blender 4.2+ [20].
- Ultimate Trim UV and generators for Blender, 3ds Max and Substance. They were free for
  non-commercial use in 2019 [2].

For Forge, the sheet can be generated rather than authored: the kit's part programs are rendered
into strips when assets are cooked.

## 2. Fighting aliasing of fine repeated geometry

### 2.1 Why it shimmers at 30–40 m

The pixel footprint at distance d is p(d) = 2·d·tan(fov_y/2)/H. Use the internal (pre-upscale)
height H. A periodic pattern aliases once its period drops below 2 px (Nyquist). A bar flickers
once it is narrower than 1 px. For a 60° vertical field of view:

| H (px) | Pixel at 40 m | Slats (7 cm) < 2 px beyond | Bars (1.8 cm) < 1 px beyond | Bar spacing (11 cm) < 2 px beyond |
|---|---|---|---|---|
| 1080 | 4.3 cm | 33 m | 17 m | 51 m |
| 1440 | 3.2 cm | 44 m | 22 m | 69 m |
| 2160 | 2.1 cm | 65 m | 34 m | 103 m |

Forge's fly camera uses a 70° vertical field of view, which brings these distances about a fifth
closer. The slat column matches the 30–40 m that was observed.

Along a street, a railing's horizontal spacing also shrinks by sin(θ) at grazing angles. At 15° the
11 cm spacing projects as 2.8 cm, so it falls below 2 px at about 13 m at 1080p.

### 2.2 What virtualised geometry does and does not do

- **Simplification is not prefiltering.** Nanite-style cluster DAGs simplify by geometric error
  [21]. Slats and bars are separate pieces, so the simplifier cannot merge them without losing the
  gaps. That changes the average colour, which shows up as a pop.
- **Epic's own documentation agrees.** For thin, aggregate geometry like foliage, it says triangles
  hurt culling and simplification, and that the detail turns sparse in the distance. Its answer,
  Nanite Voxels, is experimental: near pixel-sized voxels replace triangle clusters when that gives
  lower error [22].
- **TSR has a separate fix.** Unreal documents thin-geometry detection for TSR. It explains that a
  partly covered pixel swings between foreground and background as the jitter moves. It also
  detects edges separately for regular structures, and names railings and fences [23].
- **Ray tracing does not catch it either.** NVIDIA's adaptive ray-traced TAA finds where TAA fails,
  but it can miss sub-pixel geometry. As a remedy, the authors suggest artists produce "alternate
  level of detail representations" beyond a certain distance [24].

### 2.3 Geometry-to-texture LOD

Past a projected-size threshold, swap thin geometry for a textured card or impostor. Mipmapping
then filters the pattern correctly. Unreal's HLOD merges distant actors into proxy meshes with
baked materials [25]. Spider-Man generated impostors procedurally [5].

The owner notices LOD pops immediately, so the swap must not be visible:
- Bake the card from the same part program.
- Switch where the two forms already look alike (a slat period of about 3–4 px).
- Crossfade over a band with a temporal dither, which TAA resolves.

### 2.4 Minimum-width thin geometry ("wire AA")

Persson's phone-wire technique never lets a wire get thinner than one pixel. Below that, it fades
the wire's opacity by the ratio between the true width and the clamped width [27]. It is also
published in GPU Pro 5 (2014). It works in a vertex or mesh shader because bars are boxes with a
known axis. Opacity needs either a forward blended pass or a screen-space dither that TAA averages.

### 2.5 Alpha options

- **Alpha-to-coverage** needs MSAA, which a visibility-buffer + TAA renderer usually lacks [19].
- **Hashed alpha testing** gives stable stochastic thresholds [28], but it is patented
  (section 4).
- **For blended or dithered coverage**, plain box-filtered alpha mips are the correct average.
  Castaño's rescaling only applies to binary alpha testing [18].

### 2.6 Specular antialiasing

- **Toksvig (2004).** The shortening of averaged normals measures how much normals vary. That
  variation is used to widen and dim the highlight [29].
- **LEAN and LEADR.** They store the moments of the normal distribution in linear textures, so
  mipmapping and anisotropic filtering stay correct [30][31].
- **Kaplanyan, Hill, Patney, Lefohn (HPG 2016, best paper).** They filter the normal distribution
  function over the pixel footprint using derivatives of the normal. This catches highlights from
  curvature [26]. Kaplanyan's GDC 2016 slides show that even 16× MSAA does not fix it [32].
- **Tokuyoshi & Kaplanyan (JCGT 2021)** improve stability further and report use in Unity and
  Lumberyard [33]. Baker & Hill's "Rock-Solid Shading" is the production survey [34].

There is a limit for this kit. Box faces have no normal variation inside a triangle, so geometric
specular AA helps less than prefiltering does. Its main use here is on the normal maps of the far
forms.

### 2.7 TAA and sharpening

TAA turns spatial aliasing into flicker [35][36]. Karis's 2014 talk is the standard reference for
history clamping [37]. The FSR1 documentation asks for a noise-free input to RCAS and places it
before grain [38]. Residual flicker is high-frequency temporal noise, so sharpening raises it; that
last step is reasoning, not a published measurement.

Forge's measurement at one street view (2026-10-11): two consecutive frames differed in 4 377
pixels with RCAS on and 2 444 with it off, but the strong flicker (ꟻLIP ≥ 0.1) fell only from
1 957 to 1 828 pixels. Sharpening is not the cause. Fix the content first, then adjust TAA: lower
sharpening, or relaxed clamping on a thin-feature mask like Unreal's [23].

### 2.8 Concrete recommendation

- **Louvred shutters.**
  - Near: geometry, as now.
  - Far (slat period under about 3 px): the frame stays geometry, and the louvred field becomes one
    quad with a baked slat tile (albedo, normal, AO, optional height).
  - No alpha is needed, because the gaps are dark.
  - Fold normal variance into roughness in the normal map's mips, Toksvig-style.
  - Add POM only if an A/B test shows a difference at grazing views.

  (Shading the slats as one surface was tried in Forge and changed nothing: the shadows and AO still
  alternate slat by slat.)
- **Railings.**
  - Near: geometry.
  - Mid: bars clamped to at least 1 px with dithered coverage.
  - Far: one quad with a coverage texture, using mean-coverage mips.
  - Ray-traced shadows: any-hit against the coverage texture at a fixed mip.
- **Mullions and frames.**
  - Mid: width clamp only.
  - Far: bake the glazing bars into the window card.

## 3. Paving and ground patterns

### 3.1 Align UVs to each region, not to the world

The current top-down world projection is CGA's `world` anchoring. The fix is scope anchoring per
region [13]. CityEngine gives every street lane a normalised UV set plus two UV sets in metres,
measured along and across the lane from either end. Intersections get their own shapes with the
same normalised parameterisation [39].

For Forge:
- **Carriageways:** s = arc length along the centreline, t = signed lateral offset.
- **Pavements and facade strips:** s along the kerb or facade line.
- **Squares:** either the frame of the longest facade or oriented bounding box, or polar (r, φ)
  around a centrepiece for fan and ring motifs.

The street generator already has all these lines, so this is mostly passing data through.

### 3.2 Pattern variety

Write the patterns as shader functions of (s, t). Each returns a stone id, local coordinates and
the distance to the nearest joint. The library should include:
- running bond, stack bond, basketweave;
- herringbone (Unity's Shader Graph samples include one [40]);
- segmental arcs and fans ("pavés en éventail" / Segmentbogenpflaster);
- concentric rings and radial patterns for squares.

OpenStreetMap already tags sett patterns `arc`, `fan` and `interleaved` [41], which could drive the
choice from real data later. Real fans use setts of several sizes, from 50–60 mm up to 100–120 mm,
to keep joints even on varying radii [42]. Vary stone size along each arc to match.

Hash each stone id into tint, height, bevel and joint width. Texture bombing can add stains [44].
Joints must be filtered analytically using derivatives [45], and the pattern must fade to its mean
colour as the stone period nears 2 px. Without this the paving will shimmer the way the slats do.

### 3.3 Junctions between differently oriented patterns

- **Border courses (primary).** Real paving rarely blends orientations; it separates them. Kerbs,
  gutters and single or double rows of setts frame fields, and a single course of setts is used as
  a transition strip between surfaces [46]. In one documented fan installation, the fan edges are
  picked out in a different stone [42].

  Give every region boundary a 10–40 cm border strip from a paving trim sheet, with U = arc length
  along the boundary. Clip each field to the inside of its strip. The seam then always sits under a
  border, and strips meet at mitred corners, which is a small set of cases.
- **Intersections.** Treat each intersection as its own region with its own frame, for example arcs
  centred on the junction. CityEngine models intersections as separate shapes [39]. Houdini's Labs
  Road Generator builds crossings automatically and outputs road side lines for pavements [47]. The
  Ghost Recon Wildlands road builder grows new roads into the existing network and stamps the
  result onto the terrain [48].
- **SDF boundaries.** If the ground stays one mesh, look up a region id per pixel plus the exact
  polygon SDF [49]. These pick the pattern and draw the border band. They replace per-region
  meshes, not the border idea.
- **Decals at seams.** Use decals for drains, manholes, repairs and stains [15][16]. Splat maps are
  for wear and moss, not orientation.
- **Virtual texturing.**
  - Battlefield 3 composited spline and quad decals into a procedural virtual texture. Its
    practical limit was about 32 samples/m, with detail splatting adding 500–1000/m on top [50].
  - Far Cry 4's adaptive virtual textures reached 10 texels/cm with many decals [51].
  - Unreal's runtime virtual textures cache landscape shading, and splines and decals are
    composited into that cache [52].

  Setts need the detail layer anyway, so virtual texturing would only cache results. Leave it
  until profiling asks for it.

## 4. Patents

Checks were web searches plus the Google Patents record (status as Google states it, not a legal
opinion). "None found" means none found by web search; patent databases were not searched for it.

| Technique | Patent met | Status | Alternative |
|---|---|---|---|
| Hashed alpha testing | US 10,417,813 B2, NVIDIA (Wyman, McGuire), priority 2016-12-05 [53] | Active, anticipated expiry 2037-11-10 | Screen-space ordered (Bayer) dither + TAA, forward blending, coverage mips [18] |
| Alpha-to-coverage, virtual-sample variant | US 8,669,999 B2, NVIDIA, priority 2009-10-15 [54] | Active, adjusted expiry 2032-08-16 | Basic A2C is a core API feature (origin patents not checked); not needed without MSAA |
| Parallax texture lookup | EP 1 112 553 B1, Philips, priority 1999-07-20 [55] | Expired (lifetime), 2020 | Parallax / POM; no POM-specific patent found |
| Virtual texturing (page tables + fade factors) | US 9,349,210, ARM, priority 2012-11-30 [56] | Active, adjusted expiry 2034-01-10 | Not recommended now; a full patent check is needed before building VT |
| TAA in a multisampling pipeline | US 9,208,605, NVIDIA, priority 2003-08-29 [57] | Active, adjusted expiry 2030-01-11 | Already-shipped TAA; scope of claims not analysed |
| Trim sheets, UV snapping, kit-baked cards, HLOD | None found | – | Long-standing public practice [1][25] |
| Wire AA (minimum width + fade) | None found | – | – |
| Toksvig, LEAN/LEADR, NDF filtering (Kaplanyan et al., Tokuyoshi) | None found | – | – |
| Procedural paving, SDF borders, deferred/clustered decals | Not checked | – | – |

## Recommendations

S = 1–2 days, M = up to about a week, L = more.

1. **(S) Measure first.**
   - Add a debug view that colours each kit part by projected period or width in pixels.
   - Run A/B with RCAS on and off, and with the TAA clamp at its current setting and relaxed.
   - Confirm the table in 2.1 at Forge's field of view.
2. **(M) Shutter far form.**
   - Bake slat tiles from the part program into a generated trim sheet at cook time.
   - Switch by projected slat period, with a dithered crossfade.
   - Fully procedural.
3. **(M) Railings and mullions.**
   - Width clamp with dithered coverage in the mid range.
   - A far card with mean-coverage mips.
   - Ray-traced shadow any-hit against the coverage texture.
   - Do not use hashed alpha (patent).
4. **(S) Specular AA.** Toksvig-style roughness in the far-form normal mips, plus geometric
   normal-variance widening (Kaplanyan/Tokuyoshi).
5. **(S) TAA tuning,** only after steps 2–4 and judged by A/B: lower sharpening and relax history
   rejection on a thin-feature mask.
6. **(M) Full facade trim sheet.**
   - Strips for cornice, string course, plinth, wall field and the far forms.
   - Power-of-two strip heights, one texel density, 16× anisotropic filtering.
   - Kit UV rule: U in metres along the wall, V snapped to a strip.
7. **(M) Paving UV frames** per region from the street graph, replacing the world projection.
8. **(M) Paving pattern library** in the shader: hash variation and filtered joints, with pattern
   choice by rules (street class, square size, district).
9. **(M) Border strips** at every region boundary, plus intersection-specific patterns, so every
   orientation change sits under a border.
10. **(L, later) Seam decals.** Build virtual texturing only if profiling demands it, and only
    after a dedicated patent check.

**Assets.** Everything above can be generated: trim strips are baked from the kit programs, and
paving is procedural. The only optional downloads are photographic micro-detail for stone, stucco
and metal. ambientCG and Poly Haven publish under CC0 [58][59]. Each download still needs the
owner's approval.

## Sources

1. [talk] Olsen, "The Ultimate Trim: Texturing Techniques of Sunset Overdrive", GDC 2015. <https://gdcvault.com/play/1022323/The-Ultimate-Trim-Texturing-Techniques>
2. [blog] CG Channel, Ultimate Trim tools (2019). <https://www.cgchannel.com/2019/07/download-justen-lazarros-free-ultimate-trim-texturing-tools>
3. [talk] Benno, MacAuliffe, Mullen, Spider-Man look creation, GDC 2019. <https://gdcvault.com/play/1026495/-Marvel-s-Spider-Man>
4. [talk] Santiago, "Procedurally Crafting Manhattan", GDC 2019. <https://www.gdcvault.com/play/1025765>
5. [talk] Santiago, "Marvel's Spider-Man, meet Houdini". <https://vimeo.com/326448655>
6. [talk] Smith, Olguin, "Texturing Uncharted 4: a matter of Substance". <https://gdcvault.com/play/1023488/Texturing-Uncharted-4-a-matter>
7. [blog] SideFX, procedural buildings of The Finals. <https://www.sidefx.com/community/making-the-procedural-buildings-of-the-finals-using-houdini/>
9. [blog] 80.lv, "Arabian Afternoon: Working with Trim Sheets Efficiently" (2020). <https://80.lv/articles/arabian-afternoon-working-with-trim-sheets-efficiently>
10. [forum] Polycount, trim vs decal sheets (a single thread; the Polycount wiki was unreachable). <https://polycount.com/discussion/comment/2667525>
11. [forum] Unreal community tutorial, trim sheet workflows. <https://forums.unrealengine.com/t/community-tutorial-optimizing-massive-environments-in-ue5-aaa-trim-sheet-workflows-maya-blender-substance-ue5/2745366>
12. [docs] Houdini Labs Automatic Trim Texture. <https://www.sidefx.com/docs/houdini/nodes/sop/labs--automatic_trim_texture.html>
13. [docs] CityEngine CGA setupProjection. <https://doc.arcgis.com/en/cityengine/latest/cga/cga-setup-projection.htm>
14. [paper] NVIDIA, "Improve Batching Using Texture Atlases" (2004). <https://www.jmeiners.com/shamans/papers/graphics/texture_atlas.pdf>
15. [talk] Sousa, Geffroy, "The Devil is in the Details: idTech 666", SIGGRAPH 2016. <https://www.slideshare.net/slideshow/siggraph2016-the-devil-is-in-the-details-idtech-666/64714659>
16. [blog] Wronski, "Fixing screen-space deferred decals". <https://bartwronski.com/2015/03/12/fixing-screen-space-deferred-decals/>
17. [paper] Tatarchuk, Dynamic Parallax Occlusion Mapping, I3D 2006. <https://faculty.digipen.edu/~gherron/references/References/SurfaceDetail/Tatarchuk-POM-SI3D06.pdf>
18. [blog] Castaño, "Computing Alpha Mipmaps". <https://ludicon.com/castano/blog/articles/computing-alpha-mipmaps/>
19. [blog] Golus, "Anti-aliased Alpha Test: The Esoteric Alpha To Coverage". <https://bgolus.medium.com/anti-aliased-alpha-test-the-esoteric-alpha-to-coverage-8b177335ae4f>
20. [docs] Blender extension "Trimmer". <https://extensions.blender.org/add-ons/trimmer/>
21. [talk] Karis, Stubbe, Wihlidal, "A Deep Dive into Nanite", SIGGRAPH 2021. <https://advances.realtimerendering.com/s2021/index.html>
22. [docs] Unreal, Nanite Foliage. <https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage>
23. [docs] Unreal, TSR thin geometry detection. <https://dev.epicgames.com/documentation/en-us/unreal-engine/thin-geometry-detection-with-temporal-super-resolution>
24. [paper] Marrs et al., "Improving Temporal Antialiasing with Adaptive Ray Tracing", Ray Tracing Gems (2019). <https://casual-effects.com/research/Marrs2019ATAAGem/Marrs2019ATAAGem.pdf>
25. [docs] Unreal, World Partition HLOD. <https://dev.epicgames.com/documentation/en-us/unreal-engine/world-partition---hierarchical-level-of-detail-in-unreal-engine>
26. [paper] Kaplanyan, Hill, Patney, Lefohn, "Filtering Distributions of Normals for Shading Antialiasing", HPG 2016. <https://research.nvidia.com/publication/2016-06_filtering-distributions-normals-shading-antialiasing>
27. [blog] Persson (Humus), "Phone-wire AA". <http://www.humus.name/index.php?page=3D&ID=89>
28. [paper] Wyman, McGuire, "Hashed Alpha Testing", I3D 2017. <https://research.nvidia.com/labs/rtr/publication/wyman2017hashed>
29. [paper] Toksvig, "Mipmapping Normal Maps". <https://developer.download.nvidia.cn/whitepapers/2006/Mipmapping_Normal_Maps.pdf>
30. [paper] Olano, Baker, "LEAN Mapping", I3D 2010. <https://userpages.cs.umbc.edu/olano/papers/lean/>
31. [paper] Dupuy et al., "LEADR Mapping", SIGGRAPH Asia 2013. <https://perso.liris.cnrs.fr/victor.ostromoukhov/publications/pdf/SAsia2013-LEADR.pdf>
32. [talk] Kaplanyan, specular AA slides, GDC 2016. <https://developer.download.nvidia.com/gameworks/events/GDC2016/akaplanyan_specular_aa.pdf>
33. [paper] Tokuyoshi, Kaplanyan, "Stable Geometric Specular Antialiasing with Projected-Space NDF Filtering", JCGT 2021. <https://www.jcgt.org/published/0010/02/02/paper-lowres.pdf>
34. [talk] Baker, Hill, "Rock-Solid Shading", SIGGRAPH 2012. <https://www.selfshadow.com/talks/rock_solid_shading_v1.pdf>
35. [talk] Salvi, "An Excursion in Temporal Supersampling", GDC 2016. <https://developer.download.nvidia.com/gameworks/events/GDC2016/msalvi_temporal_supersampling.pdf>
36. [paper] Yang, Liu, Salvi, "A Survey of Temporal Antialiasing Techniques", 2020. <https://research.nvidia.com/labs/rtr/publication/yang2020survey>
37. [talk] Karis, "High-Quality Temporal Supersampling", SIGGRAPH 2014. <https://advances.realtimerendering.com/s2014/index.html>
38. [docs] AMD FidelityFX FSR1 (RCAS). <https://gpuopen.com/manuals/fidelityfx_sdk/techniques/super-resolution-spatial/>
39. [docs] CityEngine, street and intersection shape UV. <https://doc.arcgis.com/en/cityengine/latest/help/help-street-intersection-shape-uv.htm>
40. [docs] Unity Shader Graph samples (Herringbone). <https://docs.unity3d.com/Packages/com.unity.shadergraph@8.0/manual/ShaderGraph-Samples.html>
41. [docs] OpenStreetMap wiki, sett pattern/style keys. <https://wiki.openstreetmap.org/wiki/Key:sett:style>
42. [blog] PavingExpert, fan-pattern sett project. <https://pavingexpert.com/portfolio_02>
44. [paper] Glanville, "Texture Bombing", GPU Gems ch. 20. <https://developer.nvidia.com/gpugems/gpugems/part-iii-materials/chapter-20-texture-bombing>
45. [blog] Quilez, filterable procedurals. <https://iquilezles.org/articles/filterableprocedurals/>
46. [blog] Universal Paving, sett edging and transition strips. <https://universalpaving.co.uk/blogs/universal-paving/setts-cobbles-edging-borders-driveways>
47. [docs] Houdini Labs Road Generator. <https://www.sidefx.com/docs/houdini/nodes/sop/labs--road_generator.html>
48. [blog] 80.lv, Heyms, "Procedural World Building in Ghost Recon Wildlands" (2017). <https://80.lv/articles/procedural-world-building-in-ghost-recon-wildlands/>
49. [blog] Quilez, 2D distance functions. <https://iquilezles.org/articles/distfunctions2d/>
50. [talk] Widmark, "Terrain in Battlefield 3", GDC 2012. <https://media.contentapi.ea.com/content/dam/eacom/frostbite/files/gdc12-terrain-in-battlefield3.pdf>
51. [talk] Chen, "Adaptive Virtual Texture Rendering in Far Cry 4", GDC 2015. <https://gdcvault.com/play/1021760/Adaptive-Virtual-Texture-Rendering-in>
52. [docs] Unreal, Runtime Virtual Texturing quick start. <https://dev.epicgames.com/documentation/en-us/unreal-engine/runtimevirtual-texturing-quick-start-in-unreal-engine>
53. [docs] Patent record US10417813B2. <https://patents.google.com/patent/US10417813>
54. [docs] Patent record US8669999B2. <https://patents.google.com/patent/US8669999>
55. [docs] Patent record EP1112553B1. <https://patents.google.com/patent/EP1112553B1/en>
56. [docs] Patent record US9349210. <https://patents.google.com/patent/US9349210>
57. [docs] Patent record US9208605B1. <https://patents.google.com/patent/US9208605>
58. [docs] ambientCG licence (CC0). <https://docs.ambientcg.com/license/>
59. [docs] Poly Haven licence (CC0). <https://polyhaven.com/license>
