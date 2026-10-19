# Decals on static surfaces, and blend masks between ground layers (#304)

*Research note, 2026-10-19, for #304. The issue asks for decals that blend with soft alpha on static
surfaces, are lit as the surface under them, share a few textures, are cheap enough for a town and
stay steady under DLAA. At lower priority it asks for a blend mask between two ground layers, with
sharp or soft edges per pair. A research agent wrote this note from Forge's code
(`shaders/meshlet.slang`, `crates/forge-procgen/src/city/ground.rs`), from D-007, D-013, D-018,
D-021, D-026–D-028 and D-047, and from sources checked that day. No code was changed.*

**How each claim was checked:**
- **[read]**: the page was opened and read.
- **[source]**: read in an engine's public source code (Godot, Unity HDRP, Terrain3D) on GitHub.
- **[local]**: read in Forge's own code or docs.
- **[search]**: only a search summary or an abstract was seen. Either the PDF was over 10 MB, this
  machine has no PDF text tool, or the page is behind a login.
- **[recalled]**: from knowledge of the talk or paper, not re-read today.
- **[inferred]**: reasoning or an estimate, still to be measured.

> **In short.**
> 1. **Where shipped engines apply decals.** There are three places:
>    - into a G-buffer or a decal buffer, by drawing a box per decal over the depth (Unreal, HDRP's
>      opaque path);
>    - in the shading pass, from a list per cluster (DOOM 2016 and Eternal, Call of Duty, Godot 4,
>      HDRP's transparent path);
>    - baked into a virtual texture of the ground (Far Cry 4, Battlefield 3, Call of Duty's
>      terrain, Unreal's RVT).
> 2. **Forge's resolve already has what the second way needs.** Per pixel it has the position, the
>    analytic derivatives of that position, the normal and the material row. A decal there costs a
>    few loads and a texture read, and the surface's own lighting code lights it. There is no
>    geometry, so no z-fighting, and soft alpha comes free. The analytic derivatives give
>    anisotropic `SampleGrad` without the halos at depth edges that screen-space decals suffer.
> 3. **No per-frame binning for static decals.** A world grid of 4 m cells, built once on the CPU,
>    gives each pixel the few decals near it. It also stays clear of Activision's live patent on
>    hierarchical decal binning (US 12,064,688 B2, to 2042) and of AMD's on per-tile light lists
>    (US 10,803,655 B2, to 2036).
> 4. **Textures in 2-D array images, not one atlas.** At grazing angles, anisotropic filtering
>    reads past an atlas cell's border. An array layer clamps or repeats by itself. Masks are R8;
>    colour is premultiplied in its mips.
> 5. **Decals that name a material row.** D-007 says decals reference materials. A decal can be a
>    mask over a material row. A strip of grass on a road's edge is then shaded by the grass
>    ground's own row and matches the grass beside it, and physics and audio read the same row.
> 6. **The ground's blend mask** (item 4): per texel, a base layer, an overlay layer and a signed
>    distance to the edge between them, with a width and a height contrast per pair. No patent was
>    found on any part of it.
> 7. **Ruled out for a first version.** Mesh decals suit neither a visibility buffer (one surface
>    per pixel) nor cluster LOD (coarse levels move the ground under them). Runtime virtual
>    texturing is the later answer for far ground, once its live patents are checked.

## 1. What Forge has, and what #304 asks

- **The resolve** (D-021, D-026) runs a compute pass per shading class over the visibility buffer
  and rebuilds the triangle at each pixel with analytic barycentrics. `Surface` holds the normal,
  the positions in object space and the scene frame (`object_position`, `world`), their screen
  derivatives (`dpdx`, `dpdy`), the UVs when the mesh has them, and the instance.
  `standard_surface` turns a row into albedo and a normal; `sun_light` lights them (sun rays,
  probes, sky, GTAO). [local]
- **The rows** (D-026, D-047) are `GpuMaterial`, 176 bytes each. Textures sit in the global
  bindless set, which has `Texture2D` views only today. The 16× anisotropic samplers clamp or
  repeat per axis (`SamplerKind::AnisotropicWrap`). [local]
- **The layered ground** (D-028) is an R8 map with one layer id per texel: 1 m in the city, 16 MB
  for its 4 km. The four texels around a pixel weigh their layers bilinearly, and the two heaviest
  layers are shaded. A comment in `map_layers` records that shading a third layer made the pass
  take 128 registers instead of 96 and spill. [local]
- **Road markings are geometry today.** `city::ground` cuts the zebras' stripes out of the
  carriageway as pieces with their own row (D-061, "every surface once"). #256 warns that thin
  white stripes alias. [local]
- **#304's scene** is built outside this repository. It bakes every base-plus-overlay tile into a
  texture of its own: thousands of composites, with blends that stop hard at the tile's edge.
- **Earlier decisions:** D-013 names a decal atlas in the regional material set and Mikkelsen's
  surface gradients as the one rule for compositing normals; D-007 says decals reference materials
  by id. [local]

## 2. Projected decals into a G-buffer or a decal buffer

- **The method.** A decal is an oriented box, drawn as geometry. Each pixel it covers rebuilds its
  position from the depth and moves it into the box's frame. If the position is inside the box,
  the pixel reads the decal's texture at the box's x and z. The result is blended into the G-buffer
  before lighting. The usual references [search]:
  - Krassnigg, Game Engine Gems 1, 2010 [1];
  - Persson's volume decals, GPU Pro 2, 2011 [2];
  - Kim's screen-space decals in Space Marine, SIGGRAPH 2012 [3].
- **Mips.** The hardware takes its derivatives from 2×2 quads of the box's fragments. Where a quad
  straddles an edge of the scene, one pixel's position lands far away in the box, and the
  derivatives pick a much blurrier mip. The result is a halo along every silhouette that crosses a
  decal. Wronski lists the fixes [4] [read]:
  - clamping the UVs;
  - using only the top few mips;
  - several passes;
  - Forward+-style lists in the geometry pass;
  - analytic derivatives from the normal and the distance;
  - derivatives stored in the G-buffer.
- **Normals and roughness.** Fixed-function blending can only lerp each render target, and it
  cannot read the encoded normal it writes over (Wronski [4]; Frostbite's PBR notes, "PBR and
  decals" [5] [recalled]).
  - **Unreal's DBuffer**, the default since UE5: decals accumulate colour, normal and roughness,
    with their opacities, in targets of their own before the base pass. The base pass reads them
    while it shades, so each material chooses which channels respond. Emissive and AO are drawn
    separately [6] [read].
  - **HDRP** does the same for opaque surfaces. Its decal code reads four DBuffer targets [17]
    [source], and the receiving materials go through a depth prepass [7] [read].
- **Order and cost.** Unreal sorts by a "Sort Order" per decal. Cost follows screen coverage
  first, then the number of decals, and each decal is a draw (two for a large box seen from
  inside) [6][21] [read].
- **Visibility-buffer engines.** Nanite meshes receive projected decals, whose depth comes from the
  depth pass Nanite already has, but not mesh decals [6][10] [read].
- **For Forge:** a decal buffer would cost a raster pass per decal (or a screen-space pass),
  screen-sized targets of about 8–16 bytes a pixel, and the derivative problem the resolve does
  not have. It is only worth it if an inline loop does not fit the class passes' registers
  (§10.4).

## 3. Clustered decals: a list per cell, read while shading

- **DOOM (2016).**
  - **Binning.** The CPU bins decals as boxes into the frustum's clusters, as it does the lights
    and the probes: 16 × 8 tiles and 24 logarithmic slices, up to 256 decals a cluster [11] [read].
  - **Shading.** The clustered forward pass reads them from an atlas, with no draw call per decal
    [11] [read].
  - **Scale.** A Chinese write-up of the renderer shows a scene with 300 lights and 1 200 decals
    [12] [read]. The talk's slides give an 8 000² atlas and up to 4 000 decals in view. That
    figure comes from `procedural-content.md`'s search record and was not re-read [13] [search].
- **Doom Eternal (2020)** [14] [search]:
  - **Why it changed.** Larger levels put many distant decals and lights into a few big clusters.
  - **What it does.** Binning moved to the GPU: a coarse, then a fine compute raster of the
    entities' hexahedra into tiles and depth bins, inspired by Drobot, on a budget under 0.5 ms.
- **Call of Duty: Infinite Warfare** (Drobot 2017) [15] [search]: z-binning. Entities, decals
  among them, are sorted by depth; each depth bin holds a min/max index range, combined with a
  bitmask per screen tile.
- **Godot 4** (Forward+, MIT) [9][16] [source]:
  - **The list.** Decals share the cluster builder with the lights and probes: 512 elements in view
    by default. At each pixel, a screen tile's bitmask is cut by the depth slice's index range. The
    mask is made uniform across the wave (`subgroupOr`) and walked bit by bit.
  - **Sampling.** Each decal moves the pixel's position into its box. `textureGrad` takes the
    box's transform of `dFdx` and `dFdy` of the position, so mips and anisotropy stay right inside
    the loop.
  - **Blending.** Albedo is lerped by alpha, and the normal becomes
    `normalize(mix(n, n_decal, alpha))`. AO, roughness and metalness are lerped too. Fades follow
    the height in the box and the angle to the box's axis.
  - **The atlas** packs decals on blocks of 2^m texels for m mips, so its mips do not mix
    neighbours [inferred from the code].
- **HDRP's clustered path** (transparent surfaces) [17] [source]:
  - **Mips.** It picks a level by hand from the decal-space derivatives ("we can't compute LOD
    inside a dynamic loop") and samples trilinearly, so anisotropy is lost.
  - **Atlas and normals.** UVs are clamped half a texel inside each rect; normals are surface
    gradients (`SurfaceGradientFromTBN`, `SurfaceGradientFromVolumeGradient`).
- **MJP's deferred-texturing sample (2016)** [18] [read]: a G-buffer of UVs and tangent frames,
  and decal bitmasks per cluster (16 × 16 pixel tiles × 16 slices) read through bindless
  textures. With about eleven decals covering most of a GTX 970's screen, the deferred pass went
  from 1.50 to 2.00 ms (1.33 ms with the decal step branched out entirely).
- **World-space grids.** PlayCanvas bins its lights into a world-space grid over their bounds,
  indexed by the fragment's world position [19] [search]. Ikkala et al.'s hierarchical bitmask
  implicit grids (GRAPP 2024, GPL code) index tens of thousands of decals in world space in
  0.1–0.2 ms, about 2.9× faster than the decal method they compare with, rendering as fast or
  faster [20] [read].

## 4. Mesh decals

- **The method** [21] [read]:
  - **Geometry** modelled to hug the surface: a strip of grime on a kerb, a spline of road paint.
  - **Drawing.** It is translucent and drawn after the opaque pass with a small depth bias.
  - **Soft edges** come from feathering the geometry.
- **Limits in Unreal** [21] [read]:
  - no adjustable depth bias (an offset in the mesh, or one moved by world position offset);
  - no artist-specified sort order (sort by depth instead);
  - trouble with LOD and long view distances.
- **Elsewhere.** Mesh decals are not supported on Nanite meshes [10]. In Doom Eternal, mesh decals
  belong to a mesh's authoring, according to a third-party frame breakdown [search].
- **For Forge:** a visibility buffer keeps one surface per pixel. A mesh decal there has two
  options:
  - **cut out in the visibility pass:** binary alpha only;
  - **drawn after the resolve,** as the glass is (`RenderLayer::drawn_after`): a second lighting
    path, sorting, and a fight with the ground's coarse cluster levels, which move the surface
    under the decal by their error.

  Projected decals follow whatever level is drawn.

## 5. Decals baked into virtual textures

- **Unreal's RVT** [22] [read]:
  - **How it works.** The GPU renders the landscape's material, splines and decal-like materials
    into cached pages. The view is orthographic, there is no depth, and a sort priority orders the
    layers. The landscape then samples the cache.
  - **Limits.** Content must be static. A game uploads 8 pages a frame by default. Effects that
    depend on the view are approximated through the mip.
- **Far Cry.** Far Cry 4 (Chen, GDC 2015; GPU Pro 7) uses adaptive virtual textures over
  10 km × 10 km, at about 10 texels/cm, with "a massive amount of decals" [23] [search]. Far Cry
  5's terrain talk is about the terrain's GPU geometry in its abstract [24] [search]. That Far Cry 5
  kept Far Cry 4's textures is recalled, not checked.
- **Battlefield 3** (Widmark 2012) composites spline and quad decals into a procedural virtual
  texture at about 32 samples/m, with detail splatting on top, as `trim-sheets.md` records [25]
  [recalled].
- **Call of Duty.**
  - Treyarch's terrain (Black Ops 4, Warzone, Cold War) uses virtual texturing for "seamless
    blending of artistic elements" [26] [read].
  - Activision's 2023 talk builds on Far Cry 4's work. Its decals live in a streamed quad tree and
    are composited when a page is requested [27] [search].
- **Trade-offs.**
  - **For:** the cost falls on page updates, not on pixels, so it wins where decals overlap heavily
    far away.
  - **Against:**
    - a page table, feedback and a cache to manage;
    - pages that pop as they update, which the owner sees;
    - a limit on texel density near the camera;
    - live patents (§9).

  `terrain-genesis.md` §6 suggested this route for the island's evaluated ground. This note puts it
  after per-pixel decals.

## 6. Getting the look right: normals, order, mips, alpha, steadiness

- **Lit like the surface.** Every engine above changes the surface's parameters before lighting:
  albedo, normal, roughness, metalness, AO. In Forge that is between `standard_surface` and
  `sun_light`. Shadows, probes, GTAO and reflections then treat the paint as part of the asphalt.
- **Normals.** Three modes are needed:
  - **keep:** paint follows the asphalt's grain;
  - **add:** a crack;
  - **replace:** grass over the road's edge.

  Lerping and renormalising (Godot) handles only replace. Reoriented normal mapping
  (Barré-Brisebois and Hill 2012) handles add [28] [recalled]. Mikkelsen's surface gradients handle
  all three, including a decal projected from a volume [29] [search]:
  - each contribution is blended as a gradient, then resolved once;
  - HDRP uses them for decals [17], and D-013 chose them;
  - in the resolve, a row's normal v becomes a gradient with one division: `(k n − v) / k`, where
    `k = n·v` [recalled].
- **Roughness and reflectance** are lerped by alpha, in Forge's terms: `specular_power` from the
  roughness, `specular`, `reflectance`. Wet paint and puddles are decals that change roughness only
  (Godot's "albedo mix" at 0 [9]).
- **Order.** Each pixel's list is applied in painter's order: by a priority per kind, then by id.
  The priorities run ground blends, then paint, then cracks, then grime. A list built and sorted
  once gives the same pixels every run.
- **Mips and anisotropy in a compute resolve.** The decal's UV derivatives are the box's axes
  dotted with the pixel's `dpdx` and `dpdy`, which D-021 already computes analytically.
  - They stay continuous across scene edges, since no quad straddles anything, so Wronski's halo
    never appears.
  - `SampleGrad` keeps 16× anisotropy, which HDRP's clustered path gives up.
  - `MeshletRenderer::texture_lod_bias` scales them under DLSS, as for any texture.
- **Alpha that knows its mip.** A soft decal blends as `base·(1 − a) + c·a`.
  - The mips must average `c·a` and `a`, premultiplied. Otherwise the colour of transparent texels
    leaks into the edges as a fringe [30][31] [search].
  - A mask's box-filtered mips are already the right coverage. Castaño's coverage scaling applies
    to alpha tests only (`trim-sheets.md` §2.5).
  - Thin paint then fades into the asphalt's average colour with distance instead of breaking into
    dashes. Geometry stripes cannot do that, which is #256's worry.
- **Atlas or array.**
  - An atlas needs gutters. Clamping UVs half a texel inside a rect (HDRP) still lets an
    anisotropic footprint, up to 16 taps along the slant, read the neighbour at grazing angles.
    Godot's 2^m blocks protect only m mips.
  - A 2-D array image gives each layer its own clamp or repeat, so a strip decal (skirting, a kerb,
    a road's edge) can repeat along its length.
  - Forge's bindless set would need a `Texture2DArray` declaration beside its `Texture2D` ones.
- **Edges.** A box cuts its texture off at its sides. Two things stop that showing:
  - the cook keeps a transparent border around each decal;
  - the decal fades over the last centimetres of its depth, and by the angle between the surface
    and its axis (Godot's upper, lower and normal fades [9]). It then does not smear down a kerb's
    face or round an inside corner.
- **DLAA.** A decal applied in the resolve is part of an opaque surface. It has the surface's
  motion vectors, needs no reactive or transparency mask, and gives DLAA nothing to ghost. Two
  risks remain:
  - texture aliasing, which the mips above handle;
  - the highlights of normal-mapped decals at distance: put Toksvig-style roughness in the normal
    map's mips (`trim-sheets.md` §2.6).

## 7. Ground layers: splat maps, height blends, edges per pair

- **Splat maps** weigh layers by painted masks and blend them linearly.
- **Height blending** (Mishkinis 2013) adds each layer's height map to its weight and keeps
  whichever layer rises above the rest, so sand settles between cobbles. A depth parameter sets
  how soft the transition is [32] [search]. The engines' versions:
  - Unreal's `LB Height Blend` [33] [search];
  - Unity's "Height-based blend", with a "Height Transition" width in world units [34] [search].
- **A base and an overlay per texel.**
  - Terrain3D (Godot, MIT) packs each texel into 32 bits: base id (5 bits), overlay id (5), blend
    (8), UV angle and scale, a hole bit and a navigation bit [35] [read].
  - Its shader height-blends the two layers with one global sharpness:
    `exp2(sharpness·log2(w + h))` [source].
  - The Witcher 3's REDengine 3 moved away from "conventional terrain material layers and linear
    material blends" towards an automated scheme. Only the abstract was seen [36] [search].
- **Sharp edges from a coarse map.** A blend byte read bilinearly gives soft edges whose shape
  follows the texel grid.
  - Stored as a signed distance to the base/overlay edge, the same byte gives a crisp straight edge
    at any angle between texel centres (Green 2007, Valve) [37] [recalled].
  - A transition of width w is then a `smoothstep` over the distance, with w never narrower than
    the pixel's footprint. A sharp pavement edge thus stays one filtered pixel wide at a distance.
  - Where the four texels around a pixel disagree on the pair (where three surfaces meet), today's
    two-heaviest rule takes over.
- **Per pair.** A small table indexed by (base, overlay) gives the width and the height contrast:
  pavement to road 2 cm with no height, grass to dirt 60 cm with the grass's height.

## 8. The approaches side by side

| | G-buffer / DBuffer | Screen clusters | World grid in Forge's resolve | Mesh decals | Virtual texture |
|---|---|---|---|---|---|
| Normals, roughness | per-target lerp; DBuffer read by the material | anything, before lighting | anything, before lighting | per-target lerp | anything, baked |
| Z-fighting | none | none | none | depth bias, LOD | none |
| Order | draw order, sort key | list order | list order, fixed at build | depth sort | composite order |
| Mips at depth edges | halos [4] | right (fragment derivatives) | right (analytic) | right | right (cache mips) |
| Anisotropy | if derivatives are right | Godot yes, HDRP no | yes, `SampleGrad` | yes | yes |
| Cost per pixel | each covering decal | a list walk | a cell lookup and a list walk | each covering decal | one cache read |
| Cost per decal | a draw | binning every frame | nothing per frame | a draw | page re-renders |
| Memory | screen targets | grid, records, textures | grid, records, textures | meshes | page cache, page table |
| Streaming | with the decals | with the decals | a grid per world cell | with meshes | pages by feedback |
| Under DLAA | opaque, fine | opaque, fine | opaque, fine | blended: sorting, masks | pops on page updates |

## 9. Patents

Status and expiry are as Google Patents states them, which is not a legal opinion. The checks
were web searches plus the Google Patents pages read today. "None found" means none surfaced in
those searches. Family members in other jurisdictions were not checked.

| Technique | Patent met | Holder, jurisdiction | Status, expiry | What the claims cover | Alternative |
|---|---|---|---|---|---|
| Hierarchical decal binning | US 12,064,688 B2 [38]; continuation US 2024/0408486 A1 pending | Activision, US | active, 2042-07-23 | the field of view split into groups of spatial units from coarse to fine; each finer group tested only against the decals its coarser group kept; pixels shaded with the finest list (claims 1, 8, 15: a server, a system, or a computer with a display) | one level of binning; Forge's world grid, built once |
| Per-tile light lists | US 10,803,655 B2 [39]; US 12,136,165 B2 [40] | AMD, US | active; 2036-01-11 and 2033-05-13 | a light list culled per screen tile into light indices per pixel, then materials shaded with them; the continuation adds depth cells per tile and a forward pipeline | the claims name lights and screen tiles; a world grid of decals is neither |
| Vector data as screen-space decals | US 10,592,242 B2 [41] | Cesium GS, US | active, 2039-07-19 | vector data cut into subunits, each with a bounding volume and an extrusion; each rasterised as a screen-space decal, its depth tested against the extrusion | markings as mask textures, or analytic stripes in the resolve; no volume rasterised per segment |
| Virtual texturing | US 9,349,210 [42]; US 8,860,743 B2 [43]; US 10,198,848 B2 [44] | ARM; NVIDIA; Sony, US | active; 2034; 2032-08-04; 2037-01-10 | ARM: page tables with fade factors (`trim-sheets.md`); NVIDIA: a sparse-texture residency map with a minimum-LOD clamp (abstract); Sony: a terrain height map as a virtual texture with nodes aligned to pages | not proposed now; a full search before any virtual texture |
| Decal authoring on meshes | US 12,198,284 B2 [45]; CN 112598770 B [46] | Adobe, US; Fujian TQ Digital, CN | active; 2042-11-27; 2040-12-22 | Adobe: vector decals laid on a 3-D object through its UV parameterisation (description; claims not read); CN: decal UVs computed per vertex and baked into the model's texture | not used: projection at run time, no per-vertex bake |
| Box decals into a G-buffer or DBuffer | none found | — | — | — | published 2010–2012 [1][2][3] |
| Clustered shading (frustum clusters) | none found (as in D-065) | — | — | — | Olsson et al. 2012 [47] |
| Surface gradients, RNM, premultiplied mips, texture arrays | none found | — | — | — | [28][29][30] |
| Height blending, base/overlay maps, distance-field edges, texture bombing | none found | — | — | — | [32][35][37][48] |
| Hashed alpha for stochastic edges | US 10,417,813 B2 [49] (`trim-sheets.md`) | NVIDIA, US | active, 2037 | — | not needed: the resolve blends real alpha |

- **Also met, not relevant:** Google's US 8,976,169 B1 blends terrain patches by altitude; it is
  active to 2033 [50].
- **Doom Eternal's talk** (August 2020) bins decals and lights by a coarse, then a fine raster, four
  months before Activision's priority date (2020-12-30). Whether that matters is a question for a
  lawyer, not for this note.
- **A side note for D-065:** AMD's claim 1 in US 10,803,655 B2 is not limited to forward
  rendering. It may bear on the clustered light lists D-065 keeps for the street lamps. The
  houses' lamps use rooms as their lists, not screen tiles.

## 10. Proposal for Forge's first version (a proposal, not a decision)

### 10.1 What to build first

1. **Projected box decals in the resolve,** read from a world grid built once, in the standard and
   layered classes.
2. **Two kinds of decal:**
   - **row decals:** a mask over a material row (a grass or gravel edge, paint as a white row, wet
     asphalt). In the first version they apply to the layered ground only.
   - **colour decals:** albedo with alpha, plus an optional normal and roughness (cracks, grime,
     stains, signs). They apply everywhere.
3. **Then:** the ground's base/overlay/distance map (item 4), decals at the mirror rays' hits, and
   texture bombing (Glanville [48]) for grime and cracks placed by a hash per cell, with no records.

### 10.2 Data

- **`DecalKind`** (a GPU table, 32 bytes a kind):
  - its array image and layer, and its rect within the layer (small decals can share one);
  - its wrapping along u: a stamp, or a strip that repeats;
  - the row it shades, or none;
  - its channels (albedo, normal, roughness) and its normal mode (keep, add, replace);
  - its priority, tint and fades;
  - a receiver mask.
- **`Decal`** (48 bytes; 50 000 decals take 2.4 MB): the box's centre in the scene frame, its kind,
  a unit quaternion, its half extents, and the repeat count along u for strips.
- **The grid:**
  - cells of 4 m in the scene frame;
  - the CPU rasterises each box into the cells it overlaps and sorts each cell's list by
    (priority, id);
  - the non-empty cells go into an open-addressed hash of (key, first, count) over one index
    array;
  - for a town of 50 000 decals at about 3 cells each: about 0.6 MB of indices and 2 MB of slots
    [inferred];
  - later, one grid per world cell (D-037) when worlds stream.
- **Textures** in 2-D arrays:
  - R8 masks at 512² (1.4 MB a layer with its mips);
  - RGBA8 colour at 1024² (5.6 MB a layer), premultiplied in its mips;
  - normal mips renormalised, with Toksvig roughness;
  - block compression (BC4, BC5, BC7) later, as D-047 decided;
  - masks generated by Forge's own code first (stripes, crack networks from Voronoi cells, grime
    gradients), as the tileables and trims are.
- **Receivers.** A row flag (`MATERIAL_FLAG_RECEIVES_DECALS`) and the decal's receiver mask
  against the instance's, so a parked car or a crate inside a box receives nothing.

### 10.3 Where it plugs into the resolve

- **The standard class:** `apply_decals(f, s, albedo, n, specular_power, specular, reflectance)`
  runs in `shade_standard` (and `shade_uv_mapped`), after the row's surface and before `sun_light`.
- **The layered class:** in `shade_layered`, row decals compete with the map's layers for the two
  shaded rows. The lightest of three fades out, as #111 does, so the pass still shades two rows.
  Colour decals apply after the blend.
- **Per pixel:**
  1. Hash the cell of `s.world` and load (first, count).
  2. For each decal: move the position into its box, reject it if outside, and apply the fades.
  3. Sample with `SampleGrad`, the gradients being the box's axes dotted with `s.dpdx` and
     `s.dpdy`, through the anisotropic sampler (clamped, or repeating along u).
  4. Blend in list order, normals as surface gradients, resolved once at the end.
- **Bounds and debugging.** The builder caps a cell at 16 decals and warns beyond. A debug view
  (`--show-decals`) colours pixels by the number of decals walked.
- **Rays:** none in the first version. Probes and mirror rays see the bare surface.

### 10.4 Numbers to measure

- **Cost:** `shading/standard` and `shading/layered` with and without decals, 600 frames, the
  best of three, at 1600 × 900 and 1440p. The estimate is 0.05–0.15 ms at 1600 × 900 for a street
  full of decals [inferred: MJP's 0.5 ms on a GTX 970 [18], scaled to a 5070 Ti].
- **Registers** (`FORGE_SHADER_STATS`): no spill in either class. If they spill, the fallback is a
  decal pre-pass into a small screen buffer, DBuffer-style.
- **Load and memory:** decals walked per pixel (mean and maximum), cells and slots, the grid's
  build time on the CPU, GPU and texture memory.
- **Steadiness:**
  - a still camera (`--fixed-step --capture-every 10`), consecutive frames under DLAA, against the
    owner's 0.05 %;
  - a dolly at a grazing angle along a marked road, against `--ssaa`, with `imgdiff --then`;
  - `meshlets --mip-check` extended to a decal at a grazing angle.
- **The other demos:** 0 pixels changed, since none of their scenes has decals.

### 10.5 The demo

- **A lab scene after #304's:** a ground of 1 m tiles (asphalt, concrete, grass) with road
  markings, a kerb strip, cracks, and grass and gravel blended onto the road's edge; two walls with
  grime and a skirting strip; a walk along it at a grazing angle.
- **The village or the city sandbox** with 50 000 placements by rules: edge blends along every
  lane, cracks and patches by the street's age, grime at the foot of walls, and #256's crossings as
  decals against today's geometry. The F1 overlay gives the cost, the A/B measure the shimmer.

## 11. Questions for the owner

1. **Where:** decals in the resolve from a world grid (proposed), a DBuffer pre-pass, or decals
   baked into a virtual texture first?
2. **Kinds:** row decals (a mask over a material row, D-007) beside colour decals, both in the
   first version?
3. **Storage:** array images rather than one atlas, RGBA8 and R8 now, block compression later (as
   D-047)?
4. **Textures:** generated by Forge (proposed), CC0 sets (each download on your go), or the
   textures of #304's own scene?
5. **Patents:** the design avoids Activision's patent on hierarchical decal binning (to 2042) and
   AMD's on per-tile light lists (to 2036). It avoids Cesium's on vector data as screen-space
   decals (to 2039) by never rasterising a volume per marking. Do you agree? Separately, AMD's claim
   may bear on D-065's street lamps (§9).
6. **The ground's blend map** costs 2–4× the layer map's memory (32–64 MB for the city's 4 km).
   Build it now, or after the decals?
7. **The crossings:** move #256's painted stripes to decals if the A/B shows less shimmer, or keep
   them as ground pieces?
8. **Scope:** decals only on static surfaces for now (no bullet holes, nothing on movers)?

## Checked and left out

- **A DBuffer pre-pass:** the fallback if the class passes' registers spill.
- **Screen binning on the GPU every frame** (Doom Eternal, Godot, Drobot): for dynamic decals and
  lights, not static ones; its hierarchical form is under Activision's patent.
- **Mesh decals** (§4) and **baking into a virtual texture now** (§5, §9).
- **Stochastic alpha:** hashed alpha is patented, and the resolve blends real alpha.
- **Height or parallax in decals:** Godot lacks it too [9]; later, if a crack needs depth.

## Verification notes

- **Read on 2026-10-19:** [4], [6]–[11], [18], [20]–[22], [26], [35], and the Google Patents
  pages [38]–[41], [43]–[46], [50]. **In source:** [16], [17], Terrain3D's `main.glsl` and its
  MIT licence.
- **Search only:** [13] and [14] (PDFs over 10 MB); [25] and [27] (no PDF text tool on this
  machine); [1]–[3], [15], [19], [23], [24], [29]–[34], [36].
- **Recalled:** [5], [28], [37] and the surface-gradient formula. Wicked Engine's clustered decals
  (an atlas, then bindless) were seen in a search only and are not cited.

## Sources

1. [book] Krassnigg, "A Deferred Decal Rendering Technique", Game Engine Gems 1, 2010. <https://gameenginegems.com/gemsdb/article.php?id=20>
2. [book] Persson, "Volume Decals", GPU Pro 2, 2011. <https://www.gameenginegems.com/gemsdb/article.php?id=1107>
3. [talk] Kim, "Screen Space Decals in Warhammer 40,000: Space Marine", SIGGRAPH 2012. <https://history.siggraph.org/?p=140287>
4. [blog] Wronski, "Fixing screen-space deferred decals", 2015. <https://bartwronski.com/2015/03/12/fixing-screen-space-deferred-decals/>
5. [course] Lagarde, de Rousiers, "Moving Frostbite to Physically Based Rendering", SIGGRAPH 2014. <https://www.ea.com/frostbite/news/moving-frostbite-to-pb>
6. [docs] Epic, "Decal Materials in Unreal Engine". <https://dev.epicgames.com/documentation/en-us/unreal-engine/decal-materials-in-unreal-engine>
7. [docs] Unity HDRP 15, "Decals". <https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@15.0/manual/Decal.html>
8. [docs] Unity HDRP 15, "HDRP Asset" (Decals). <https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@15.0/manual/HDRP-Asset.html>
9. [docs] Godot, "Using decals". <https://docs.godotengine.org/en/stable/tutorials/3d/using_decals.html>
10. [docs] Epic, "Nanite Virtualized Geometry". <https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-virtualized-geometry-in-unreal-engine>
11. [blog] Courrèges, "DOOM (2016) – Graphics Study". <https://www.adriancourreges.com/blog/2016/09/09/doom-2016-graphics-study/>
12. [blog] Gcores, a study of DOOM's renderer (Chinese). <https://www.gcores.com/articles/180790>
13. [talk] Sousa, Geffroy, "The devil is in the details: idTech 666", SIGGRAPH 2016. <https://advances.realtimerendering.com/s2016/Siggraph2016_idTech6.pdf>
14. [talk] Geffroy, Gneiting, Wang, "Rendering the Hellscape of Doom Eternal", SIGGRAPH 2020. <https://advances.realtimerendering.com/s2020/RenderingDoomEternal.pdf>
15. [talk] Drobot, "Improved Culling for Tiled and Clustered Rendering in Call of Duty: Infinite Warfare", SIGGRAPH 2017. <https://www.activision.com/cdn/research/2017_Sig_Improved_Culling_final.pdf>
16. [code] Godot, `scene_forward_clustered.glsl` and `texture_storage.cpp` (MIT). <https://github.com/godotengine/godot/blob/master/servers/rendering/renderer_rd/shaders/forward_clustered/scene_forward_clustered.glsl>
17. [code] Unity HDRP, `DecalUtilities.hlsl`. <https://github.com/Unity-Technologies/Graphics/blob/master/Packages/com.unity.render-pipelines.high-definition/Runtime/Material/Decal/DecalUtilities.hlsl>
18. [blog] Pettineo, "Bindless Texturing for Deferred Rendering and Decals", 2016. <https://therealmjp.github.io/posts/bindless-texturing-for-deferred-rendering-and-decals/>
19. [docs] PlayCanvas, "Clustered Lighting". <https://developer.playcanvas.com/user-manual/graphics/lighting/clustered-lighting/>
20. [paper] Ikkala, Lauttia, Jääskeläinen, Mäkitalo, "Hierarchical Bitmask Implicit Grids for Efficient Point-in-Volume Queries on the GPU", GRAPP 2024. <https://webpages.tuni.fi/vga/publications/HBIG.html>
21. [docs] Epic, "Using Mesh Decals in Unreal Engine". <https://dev.epicgames.com/documentation/en-us/unreal-engine/using-mesh-decals-in-unreal-engine>
22. [docs] Epic, "Runtime Virtual Texturing in Unreal Engine". <https://dev.epicgames.com/documentation/en-us/unreal-engine/runtime-virtual-texturing-in-unreal-engine>
23. [talk] Chen, "Adaptive Virtual Texture Rendering in Far Cry 4", GDC 2015. <https://gdcvault.com/play/1021760/Adaptive-Virtual-Texture-Rendering-in>
24. [talk] Moore, "Terrain Rendering in Far Cry 5", GDC 2018. <https://gdcvault.com/play/1025480/Terrain-Rendering-in-Far-Cry>
25. [talk] Widmark, "Terrain in Battlefield 3", GDC 2012. <https://media.contentapi.ea.com/content/dam/eacom/frostbite/files/gdc12-terrain-in-battlefield3.pdf>
26. [talk] Hooker, "Boots on the Ground: The Terrain of Call of Duty", 2021. <https://research.activision.com/publications/2021/09/boots-on-the-ground--the-terrain-of-call-of-duty>
27. [talk] Etienne, "Large Scale Terrain Rendering in Call of Duty", SIGGRAPH 2023. <https://advances.realtimerendering.com/s2023/index.html#CODTerrain>
28. [blog] Barré-Brisebois, Hill, "Blending in Detail", 2012. <https://blog.selfshadow.com/publications/blending-in-detail/>
29. [paper] Mikkelsen, "Surface Gradient–Based Bump Mapping Framework", JCGT 9(3), 2020. <https://jcgt.org/published/0009/03/04/>
30. [docs] NVIDIA, "Alpha Blending: To Pre or Not To Pre". <https://developer.nvidia.com/content/alpha-blending-pre-or-not-pre>
31. [blog] Courrèges, "Beware of Transparent Pixels", 2017. <https://www.adriancourreges.com/blog/2017/05/09/beware-of-transparent-pixels>
32. [blog] Mishkinis, "Advanced Terrain Texture Splatting", 2013. <https://www.gamedeveloper.com/programming/advanced-terrain-texture-splatting>
33. [docs] Epic, "Landscape Material Expressions" (LB Height Blend). <https://dev.epicgames.com/documentation/en-us/unreal-engine/landscape-material-expressions-in-unreal-engine>
34. [docs] Unity, "Terrain Lit shader" (Height-based blend). <https://docs.unity3d.com/Manual/urp/shader-terrain-lit.html>
35. [docs] Terrain3D, "Control Map Format" (MIT). <https://terrain3d.readthedocs.io/en/latest/docs/controlmap_format.html>
36. [talk] Gollent, "Landscape Creation and Rendering in REDengine 3", GDC 2014. <https://www.gdcvault.com/play/1020394/Landscape-Creation-and-Rendering-in>
37. [paper] Green, "Improved Alpha-Tested Magnification for Vector Textures and Special Effects", SIGGRAPH 2007. <https://cdn.akamai.steamstatic.com/apps/valve/2007/SIGGRAPH2007_AlphaTestedMagnification.pdf>
38. [patent] US 12,064,688 B2, Activision. <https://patents.google.com/patent/US12064688B2/en>
39. [patent] US 10,803,655 B2, AMD. <https://patents.google.com/patent/US10803655B2/en>
40. [patent] US 12,136,165 B2, AMD. <https://patents.google.com/patent/US12136165B2/en>
41. [patent] US 10,592,242 B2, Cesium GS. <https://patents.google.com/patent/US10592242B2/en>
42. [patent] US 9,349,210, ARM. <https://patents.google.com/patent/US9349210>
43. [patent] US 8,860,743 B2, NVIDIA. <https://patents.google.com/patent/US8860743B2/en>
44. [patent] US 10,198,848 B2, Sony Interactive Entertainment. <https://patents.google.com/patent/US10198848B2/en>
45. [patent] US 12,198,284 B2, Adobe. <https://patents.google.com/patent/US12198284B2/en>
46. [patent] CN 112598770 B, Fujian TQ Digital. <https://patents.google.com/patent/CN112598770B/en>
47. [paper] Olsson, Billeter, Assarsson, "Clustered Deferred and Forward Shading", HPG 2012. <https://www.cse.chalmers.se/~uffe/clustered_shading_preprint.pdf>
48. [book] Glanville, "Texture Bombing", GPU Gems, ch. 20, 2004. <https://developer.nvidia.com/gpugems/gpugems/part-iii-materials/chapter-20-texture-bombing>
49. [patent] US 10,417,813 B2, NVIDIA. <https://patents.google.com/patent/US10417813>
50. [patent] US 8,976,169 B1, Google. <https://patents.google.com/patent/US8976169B1/en>
