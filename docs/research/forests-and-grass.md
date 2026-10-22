# Seamless forests and dense grass: the ladder per tree, its rays, and grass near the camera

Research for #302 (forests that stay seamless as the camera moves) and #300 (dense grass near the
camera), 2026-10-19. It extends [vegetation-materials.md](vegetation-materials.md) (§2 the
rendering ladder, §3 grass; D-013) and [flora.md](flora.md) (D-063); their entries are referenced,
not repeated. A research agent wrote it; pages were read on 2026-10-19. None of this is legal
advice.

**How each claim was checked:**
- **[read]**: the page was opened and read (some through a public reader proxy, said where).
- **[source]**: read in public source code.
- **[local]**: read in Forge's repository, on this machine (`vulkaninfo`), or reported in #300/#302
  by the scene built outside the repository.
- **[search]**: only a search-result summary or a secondary write-up was seen.
- **[recalled]**: from knowledge of the work; not re-read today.
- **[inferred]**: reasoning or an estimate.

> **In short.**
> 1. **Every engine with a public source draws a tree as a ladder of representations and hides
>    the crossings one of three ways:** a switch below a pixel of error inside a cluster
>    hierarchy (Nanite, including its foliage voxels), a dithered cross-fade over a band (Unreal's
>    dithered LOD transition, Unity's cross-fade, CryEngine's dissolve, SpeedTree's fade to
>    billboards), or morphing (SpeedTree's branches and shrinking leaves). Nothing public says how RDR2, the
>    Witcher 3, Far Cry, Warzone, MSFS, KCD2, Death Stranding 2, Enshrouded or Battlefield 6 do it.
> 2. **Leaf cards cannot ride Forge's cluster DAG as they are.** Open quads are all border, and the
>    DAG locks borders. Nanite's answers are area preservation (the remaining leaves dilate) and,
>    since UE 5.7, voxel bricks that take over when their error is lower. A closed crown shell
>    simplifies like any closed mesh. So the ladder is: cards near, a closed crown shell mid,
>    a hemi-octahedral impostor far, chosen per tree from its projected size, with a dithered
>    crossing that repeats with DLAA's 8 jitter phases.
> 3. **The rays' cost is the cards' any-hit, and Forge pays it on every triangle of a tree.** One
>    masked row makes the whole instance non-opaque (`FORCE_NO_OPAQUE`). Splitting each BLAS into
>    an opaque geometry and a masked one is the first, cross-vendor fix. Opacity micromaps halved
>    Indiana Jones' jungle trace (7.90 → 3.58 ms). They are now `VK_KHR_opacity_micromap` (Vulkan
>    1.4.351, May 2026). This machine's RTX 5070 Ti exposes them; AMD support was not found.
> 4. **Rays should see a fixed representation per tree, not the drawn one.** Probes and mirrors
>    get the closed crown proxy (#297). Shadow rays pick cards or proxy by their origin's distance
>    from the camera, through two TLAS instances per tree and two mask bits. The static TLAS then
>    never changes as the camera moves. Epic found the same: a simplified "poly soup" proxy gave
>    better indirect light for foliage than the exact geometry.
> 5. **Grass: a GPU pass from a density map, near only, opaque blades, no rays.** Placement is a
>    pure function of the map (cells, densities, region seeds), cached with the region. Blades are
>    real triangles (Fortnite, Black Ops 4: no alpha cards) with lighter tips over a dark base. They
>    keep at least one output pixel of width, with the missing coverage dithered as Forge's
>    gratings do. The ground's grass layer takes over beyond about 35 m.
> 6. **Patents that matter.** Pixar's US 7,973,789 B2 (active to about 2028-11-10) claims
>    representing a model by a modified subset of its elements: fewer, larger leaves or blades
>    (Nanite's Preserve Area, AMD's grass sample). Ask before using it. Also NVIDIA's micromap
>    hardware claims (US 12,475,631 B2, to 2043), its hashed alpha (to 2037) and its image-loss
>    simplification (US 11,450,077 B2, to 2041). Intel's comparison-mask LOD in rays
>    (US 11,989,817 B2, to 2042) is also live. Pixar's stochastic LOD (US 6,300,956 B1) expired in
>    2018, so a dithered crossing is free. Sony Online's flora-around-the-avatar claim (to 2028)
>    needs the owner's word on the grass pass.

**Contents**

1. [What Forge has](#1-what-forge-has)
2. [Forests at every distance](#2-forests-at-every-distance)
3. [Foliage and cluster LOD](#3-foliage-and-cluster-lod)
4. [Foliage in rays and GI](#4-foliage-in-rays-and-gi)
5. [Dense grass](#5-dense-grass)
6. [An instance draw distance with a fade](#6-an-instance-draw-distance-with-a-fade)
7. [Patents](#7-patents)
8. [A proposal for Forge](#8-a-proposal-for-forge)
9. [Questions for the owner](#9-questions-for-the-owner)
10. [Checked and left out](#10-checked-and-left-out)
11. [Verification notes](#verification-notes)
12. [Sources](#sources)

---

## 1. What Forge has

- **D-013's ladder, not built** [local]: 0–40 m alpha-tested geometry, 40–150 m billboard-cloud
  cards, 150–600 m hemi-octahedral impostors with a light-facing shadow pass and an ellipsoid ray
  proxy, then a lit canopy volume. Grass "as in Ghost of Tsushima": GPU blades from a hash.
- **D-063's flora** [local]: species grown into branch graphs (`forge_flora::graph::TreeGraph`:
  nodes with radii, leaf clusters with a position, direction and size). It also places them region
  by region, never around the player, and instances a few grown individuals per species and age
  class.
- **The trees drawn today** (`demos/city-plan/src/lib.rs`) [local]:
  - `grown_mesh`: wood 5 cm thick or more as seven-sided limbs. The leaf clusters are gathered on a
    grid (a sixth of the crown's width, an eighth for broad leaves) into lumps.
  - Each lump: a 20-triangle core at 0.2 of its radius (`CROWN_CORE`) and 6–16 leaf cards (a
    shrub's 4) cut from the leaf atlas, with normals bent out from the crown's middle.
  - `grown_proxy` (#297): the wood and each lump as a closed shell at 0.8 of its radius
    (`CROWN_PROXY`), section 3, opaque. Only the probes' rays see it, through #301's bits.
- **What sees an instance** (#301) [local]: four flag bits (no raster, no shadow rays, no probe
  rays, no mirror rays). The TLAS build turns them into an 8-bit mask: `RAY_SHADOW` 1,
  `RAY_PROBE` 2, `RAY_MIRROR` 4. Five mask bits are free.
- **The cluster DAG** (`forge-geom/src/lod.rs`) [local, source]:
  - meshoptimizer 0.25, vendored by the `meshopt` 0.6.2 crate. Groups of 8 clusters are halved per
    level.
  - The vertices shared between groups are locked by Forge's own table, and every simplification
    also passes `LockBorder`, which locks the topological border of the group's subset. A leaf card
    is an open quad: all four corners are border, so it never simplifies.
  - A group that keeps more than 85 % of its triangles stalls, and its clusters become roots. This
    is #303's "cards and blade strips never simplify".
- **The rays** (`shaders/meshlet.slang`, `forge-gpu/src/accel.rs`) [local]:
  - One BLAS per mesh, from the finest DAG cut within 40 000 triangles, its geometry flagged
    `OPAQUE`.
  - `tlas_instances_main` sets `FORCE_NO_OPAQUE` on any instance with a masked row. Then every
    triangle of that instance, wood and cores included, goes through the query loop's candidate
    test (`ray_candidate_kept`), which samples the alpha at the finest mip.
- **The raster's cut-outs** [local]: a second pipeline with `discard` and no face culling, so the
  other clusters keep early depth.
  - Gratings already keep or drop pixels by `spatio_temporal_noise(pixel, noise_frame)`: R2 along
    a Hilbert curve over 64 × 64 tiles.
  - `noise_frame` is the frame modulo the jitter phases (8 under DLAA; D-024: 8 × (output /
    render)²), so a still view converges in one cycle.
- **The instance record** [local]: 64 bytes, one uniform `scale`; `add_instance_with_material`
  keeps only the x scale (#302). Movers live in a second TLAS rebuilt every frame. When any mover
  exists, every ray also queries it (#299).
- **The visibility id** [local]: `visible slot << 7 | triangle`. Cooked clusters hold up to 64
  vertices and 124 triangles.
- **The outside scene's measurements** (RTX 5070 Ti, a town view; #302, #300) [local]:
  - Cards within 25 m of streets and buildings, opaque stand-ins beyond (lumps as closed bumped
    shells with an opaque canopy texture), over a 15 m random blend.
  - With every tree a stand-in, `gi/probe rays` went 2.01 → 0.98 ms and the frame 5.67 → 4.16 ms.
  - Stand-ins shimmer slightly less than cards: 4.3 % against 4.5 % of pixels unstable in a DLAA
    turn against SSAA.
  - Grass: one 1 m patch per grass square, placed once from its map. A CPU pool of movers shows
    the patches within 35 m and a denser layer within 12 m; each patch shrinks to nothing over its
    last metres. Thin, contrasty far blades doubled the shimmer.

## 2. Forests at every distance

| Engine / game | Near | Mid | Far | How crossings are hidden | Rays and GI | Sources |
|---|---|---|---|---|---|---|
| UE 5.7–5.8 Nanite Foliage (Witcher 4 demo, 2025) | assemblies of instanced parts (branches) as Nanite triangles, skinned for wind | the same cluster hierarchy | 4×4×4 voxel bricks, ≤ 128 a cluster, a normal distribution per voxel | per-cluster switch where the voxels' error is the lower; no blend described | HW RT: a fallback or "Ray Tracing Proxy" mesh (5.7+), with an over-occlusion bias for Lumen | [1][7][8] [read]; demo [9] [search] |
| UE 5 classic foliage, Fortnite Ch. 4 (2023) | Nanite trees, ~300–500 k vertices, opaque leaf triangles | Nanite with Preserve Area | octahedral impostors (FNBR 12×12 frames; mobile, HLOD) | Nanite: sub-pixel; non-Nanite: the dithered LOD transition flag | fallback mesh | [3][10] [read]; [42] [search] |
| SpeedTree runtime (Unity docs) | 3D tree, full wind | branches scaled to their centre line, fewer and bigger leaves | 360° billboards plus a top-down one, lit dynamically | vertex LOD morph; 3D alpha fades out while the billboard fades in (alpha to coverage) | the top-down billboard gives the better shadow under a high sun | [12] [read] |
| Unity HDRP | LOD meshes | LOD meshes | billboard | cross-fade by a dither: a spatially varying hash, not a time-varying one, for TAA | — | [14] [read] |
| Godot 4 | visibility ranges | | | hysteresis margins; alpha fade, or a dithered material | — | [15] [read] |
| CryEngine 2–3 (KCD2's family) | vegetation meshes | | sprites (impostors) | a dissolve for LOD↔LOD and LOD↔sprite | — | [16] [search] |
| Horizon Zero Dawn / Forbidden West (Decima) | SpeedTree-made LODs; HFW draws foliage through a visibility buffer | LOD 3 | LOD 4–5 as crossed planes | not said; shadow LOD by distance popped, fixed by one shadow LOD per cascade | — | [17] [search]; [18] [read] |
| Ghost of Tsushima | trees: no public detail | | | | grass: see §5 | [19][20] [read: descriptions] |
| Call of Duty BO4 Blackout | an impostor system improved for vegetation | | | not said | grass as its own system | [21] [read] |
| Assassin's Creed Shadows (Anvil) | GPU-driven; micropolygon detail on temples and castles | | point cloud to 8 km, then prebuilt views | the press saw vegetation LOD changes now and then | RT GI; dense vegetation named as a hard case | [22] [read]; press [search] |
| Alan Wake 2 (Northlight) | mesh-shader clusters, GPU-skinned vegetation | | | | the vegetation is in the RT scene in full | [23] [read] |
| Avatar (Snowdrop) | GPU-culled plants | | | | custom console BVH; RT reflections use visibly reduced vegetation meshes | [24] [read/search] |
| Indiana Jones (idTech, path tracing) | alpha-tested foliage | | | | OMMs, compacted dynamic BLASs | [27] [read] |

- **Unreal 5.7–5.8, Nanite Foliage** (Experimental) [1] [read]:
  - **Assemblies:** up to 65 k part instances, their transforms computed during cluster culling. The
    Witcher 4 demo's largest tree went from about 3.5 GB to about 29 MB.
  - **Voxels:** clusters voxelised into 4×4×4 bricks, at most 128 a cluster. A cluster switches
    from triangles to voxels where the voxels' simplification error is the lower. Voxel clusters
    are binned apart and rasterised front to back in depth buckets.
  - **Voxel shading:** each voxel holds a distribution of normals, sampled stochastically per pixel
    into the G-buffer. Epic says a later release will shade the distribution directly, with less
    noise.
  - **Wind:** skinning replaces world-position offset, for tight culling bounds and the
    fixed-function rasteriser: 100 000 bones in about 0.1 ms. Animation stops below a minimum
    screen size (0.1 by default).
  - **Scale:** the example tree has about 41 M triangles, 2 160 part instances and 850 bones. The
    demo has about 500 k instances of dozens of tree variants.
  - **What the page leaves out:** no blend between triangles and voxels, no fixed distance, and
    nothing on ray tracing.
- **Fortnite Chapter 4** [3] [read via proxy]:
  - Masked materials cost more in the base pass (barycentrics recomputed per pixel, overdraw in the
    alpha's empty space). For trees and grass, more triangles with opaque materials usually ran
    faster.
  - Simplification thinned the canopies: a leaf cannot go below one triangle, so leaves had to go.
    **Preserve Area** hands the lost area to the remaining triangles by dilating the open boundary
    edges. Enable it only on the foliage that needs it.
  - Wind was baked from a Houdini simulation into a texture (one position and quaternion lookup a
    vertex, rigid parts only), and switched off beyond an artist's distance.
- **Brucks' octahedral impostors (FNBR)** [10] [read via proxy]:
  - The hemi-octahedron is the layout for most trees: 12×12 frames for the main pine, 8×8 for
    small trees.
  - Three frames are blended per pixel, which blurs slightly but does not pop. A single depth
    offset (like bump offset) adds parallax; on sparse, busy trees it shows noisy artefacts.
  - Atlases: 2048² base colour and alpha plus 1024² normal and depth, about 6.8 MB a tree. The
    dithering needs temporal AA to resolve.
- **SpeedTree** [12] [read]:
  - A float LOD state: 1 → 0 scales the 3D detail away, 0 → −1 fades the 3D tree out while the
    billboard fades in (smooth with alpha to coverage), −1 is the billboard alone.
  - Billboards are picked by the camera's azimuth and lit dynamically. A top-down billboard
    cross-fades in when the camera looks down and casts the better shadow under a high sun.
  - Licence: Indie $19 a month (under $100 k revenue), Pro $899 a year, a runtime SDK under
    licence [13] [search]. Not something Forge links.
- **Unity** [14] [read]: SpeedTree's vertex interpolation, otherwise "Cross Fade" (a blend factor
  in `unity_LODFade`). Lagarde (2018): "varying the pattern with time confuses the TAA, increasing
  the amount of noise", so HDRP's fade uses a spatially varying hashed pattern.
- **Godot** [15] [read]: visibility ranges with begin/end margins for hysteresis. Fading is alpha
  only. A dithered material is the faster alternative with no sorting glitches, at the price of a
  noisy pattern while it crosses.
- **Horizon Zero Dawn** (Sanders, GDC 2018) [17] [search: an AI summary of the video]:
  - LOD 4 and 5 are crossed planes.
  - Shadow LOD picked by distance to the camera popped visibly. The fix was one shadow LOD for every
    object in a cascade, with shadow casters separate from the drawn meshes.
- **Black Ops 4's Blackout** (Pesce, first-hand) [21] [read]: placement was precomputed per tile as
  bitmask layers; real-time placement was dropped. A researcher reworked the geometry LOD and
  impostor system for vegetation; details not given.
- **Assassin's Creed Shadows** (GDC 2025, PCGH's report) [22] [read]:
  - Grass, bushes and trees send about 1.5 billion triangles to the GPU, about 6.8 million after
    culling.
  - Distance bands of 96, 128 and 384 m, then 1–4 km with less detail, a point cloud to 8 km,
    then prebuilt views.
  - Another PCGH piece saw vegetation LOD changes now and then [search]. The SIGGRAPH 2025 RT talk
    names dense vegetation as a challenge; its PDF (over 10 MB) could not be read.

**What the survey says** [inferred]:
- **The crossing is the issue's whole difficulty**, and only the cluster hierarchy avoids it. A
  dithered band hides a switch the eye would see. It costs both representations drawn for the
  trees in the band, and it needs temporal AA to resolve the dither (Brucks, Lagarde).
- **Engines with the budget moved leaves from masked cards to opaque triangles** (Fortnite, Nanite
  Foliage). Masked cards cost twice: overdraw and per-pixel tests in the raster, any-hit in the
  rays.
- **Impostors stay the far form wherever Nanite is not used.** Hemi-octahedral, 8×8–12×12 frames,
  three frames blended, a depth offset.

## 3. Foliage and cluster LOD

**Why cards stay** [local, source]: `meshopt_SimplifyLockBorder` keeps every vertex on the
topological border in place. A card's four corners are all border. Two coincident sheets
(front and back) collapse into each other at 1–2 cm of error (#303) but are still a card. Each
group of cards then keeps more than 85 % of its triangles, stalls, and becomes a root. A tree of
cards is drawn at full detail at every distance.

**What meshoptimizer offers** (0.25 vendored [source]; upstream at v1.3 [37] [read]):
- `LockBorder` is needed only where an open edge is a seam to keep. Forge already locks the
  vertices shared between groups with its own table, so a foliage mesh could simplify without the
  flag [inferred].
- `Prune` removes disconnected parts during simplification, whatever the topology inside them.
  It drops small cards whole, which thins a crown unless something compensates.
- `simplifySloppy` (with vertex locks) ignores topology and merges nearby separate parts; it is
  rougher. `Permissive` collapses across attribute seams (Forge uses it for sections).
- `simplifyWithUpdate` moves the vertices and attributes it keeps, which collapse-only simplification
  cannot.

**How others simplify leaves:**
- **Prune and compensate:**
  - Cook et al. (Pixar, SIGGRAPH 2007) removed a random share of the leaves and scaled the rest by
    the square root of the inverse of the kept fraction, adjusting contrast too [39] [search].
  - Gasch et al. (2018) pruned the leaf whose removal changed 20 views least and enlarged its
    nearest neighbour [38] [read]. On an English oak keeping 10 % of 20 376 leaves, their view
    error was 5.31 % against 32.25 % for random pruning; the preprocessing took 4 010 s.
  - Nanite's Preserve Area dilates the open boundary edges [3] [read].
  - All of these are near Pixar's live patent (§7).
- **Change representation:**
  - Nanite's voxels switch when their error is lower [1] [read].
  - Loubet & Neyret 2017 split each scale into macroscopic surfaces (kept as meshes) and
    sub-resolution detail (a microflake volume) [41] [search].
  - SGGX microflakes (Heitz et al. 2015) are the usual distribution of normals in such voxels
    [recalled].
- **Fit by image loss:** Hasselgren et al. 2021 (NVIDIA) optimise textured quads against renders
    of the reference; 0.4 % of the triangles for a foliage example [40] [search]. Patented (§7).
- **Neural prefiltering** (Weier et al. 2023) learns intra-voxel transport for trees such as the
  Moana bay cedar. It is interactive in a path tracer, not a raster LOD [41] [search].

**What Forge's DAG would need** (#303's per-mesh options) [inferred]:
1. **Open borders free for foliage meshes**, the group locks kept, so card strips and blade strips
   can collapse.
2. **`Prune` behind an option**, for twigs and small cards that fall below the error.
3. **An authored parent.** The generator supplies a group's coarser form instead of meshoptimizer's
   simplification: a branch's cards, then the branch's lump shell, then the crown's shell.
   - Cards are disjoint pieces. No neighbour shares their vertices, so any group of them can be
     replaced by any closed form without cracks.
   - The DAG's rule ("draw when `project(parent) > threshold >= project(self)`") holds for any
     monotonic errors and spheres.
4. **An appearance error for that parent, not only a geometric one.**
   - Take a card standing 0.3 m off its lump's shell. At 1 px of error (1600 px across 90°,
     800/d px per metre), the shell would replace the cards at about 240 m.
   - The outside scene switched at 25 m. A switch that early is a visible change the DAG's
     sub-pixel rule does not cover. Either the error is set by hand (a pop spread lump by lump),
     or the crossing is dithered (§8).
5. **No area compensation while Pixar's patent lives** (§7), unless the owner decides otherwise.

## 4. Foliage in rays and GI

**What costs** [read unless marked]:
- Epic: geometry with many small details, such as leafy trees and bushes, hurts traversal [6].
- Indiana Jones' jungle (26.9 M triangles in the structure, RTX 5080, 1080p internal) [27]:
  - The any-hit shader took 17 % of `TraceMain`'s samples.
  - Switching between traversal and the any-hit shader cost more than the shader's own work.
- **Forge** [local]: the instance-level `FORCE_NO_OPAQUE` sends every triangle of a tree (wood and
  cores too) through the candidate loop.
  - In the outside scene's measurement the probes' rays met the card trees. This loop probably
    explains much of its 2.01 → 0.98 ms with stand-ins [inferred].
  - In city-plan since #297, a tree's instance is hidden from the probes and a proxy instance is
    seen by them alone (`Visibility::PROBES`). The cards still meet the sun's, the lamps' and the
    mirrors' rays, so that is where the loop's cost now is [local, inferred].
  - The same code builds each tree's transform with a non-uniform scale (crown across, height,
    stretch), of which `add_instance_with_material` keeps the x (#302) [local].
- **The cross-vendor fix** [inferred]: two geometries per BLAS, the opaque rows with
  `VK_GEOMETRY_OPAQUE_BIT_KHR` and the masked rows without, and no `FORCE_NO_OPAQUE` on the
  instance. The loop then sees only card triangles. Core Vulkan, so the RX 9070 XT runs it.

**Opacity micromaps:**
- **What they do** [28][29] [read]: a bit mask over a uniform subdivision of each triangle.
  - 2-state: opaque or transparent.
  - 4-state: adds two "unknown" states that call the any-hit; a ray or instance flag forces 4-state
    data to act as 2-state.
  - OMM arrays are stored apart from the BLAS and shared.
- **Indiana Jones** [27] [read]:
  - 4-state OMMs (conservative) for shadow rays. Indirect rays force 2-state and skip the alpha
    test. Subdivision level 6.
  - Baked offline on the CPU: some took over 30 minutes, some hours. A 0.5 MB cap a micromap and a
    128 MB budget a map.
  - `TraceMain` 7.90 → 3.58 ms, `SharcUpdate` 1.99 → 0.89, `SunTracing` 0.83 → 0.71.
  - Compacting the never-rebuilt dynamic vegetation BLASs took them from 1027 to 606 MB.
- **Vendors:**
  - Microsoft (2026-02-26): every NVIDIA ray-tracing GPU supports OMMs, and other vendors are to
    follow over time [28] [read]. Alan Wake 2 cut its ray cost by a third with OMMs and SER [28]
    [read].
  - `VK_EXT_opacity_micromap` (2022; contributors from NVIDIA, Intel, AMD) is promoted to
    `VK_KHR_opacity_micromap` [29] [read], released in Vulkan 1.4.351 in May 2026 [29] [search].
    Per the registry, with only the KHR extension a ray query uses the micromaps only when the
    shader declares `OpacityMicromapIdKHR` [read].
  - **This machine** [local]: the RTX 5070 Ti (driver 617.42) exposes both extensions, plus
    `VK_NV_cluster_acceleration_structure` and `VK_NV_partitioned_acceleration_structure`. The
    RDNA 2 iGPU exposes none of them.
  - **RDNA 4:** no source found [search].
  - NVIDIA's OMM SDK is at 1.10.0, with CPU and GPU bakers and Vulkan support. The repository page
    did not state its licence [30] [read].
- **For Forge** [inferred]: an optional speed-up for the card geometry, by D-008's T3 ("opacity
  micromaps"). It can wait for the BLAS split's numbers and the owner's word (§9). The fallback is
  the candidate loop, so the image must be the same either way.

**Proxies for the rays:**
- **Unreal** [read]:
  - Hardware RT traces a Nanite mesh's fallback by default ("Fallback Relative Error") [2][5].
  - 5.7–5.8 add a Ray Tracing Proxy per mesh: a share of triangles, a relative error, and a
    `foliage_over_occlusion_bias` from 0 (none) to 1 (full) for Lumen's GI [7].
  - An Epic staff answer (2026-07-13) [8]: the bias was meant to give a simplified "poly soup"
    proxy, and in hardware-RT Lumen such a proxy lit aggregate or voxelised geometry better than
    the exact drawn representation. That is #297's finding, from Epic.
- **Avatar:** RT reflections use visibly reduced meshes for fine vegetation [24] [search].
- **Alan Wake 2:** its GPU-skinned vegetation is in the RT scene in full [23] [read].
- **Over-occlusion** [inferred]: a closed opaque crown stops all the sky light, and a real one lets
  some through. Three measures, from cheap to dear:
  - a shell smaller than the crown (#297's 0.8);
  - a stochastic transmittance: the candidate loop passes a hit with the crown's gap fraction,
    hashed per ray;
  - D-013's analytic crown: an AABB in the BLAS with an ellipsoid and a leaf density, its chord
    giving Beer–Lambert transmittance, as the jelly does. Ray queries take procedural candidates in
    the same loop [recalled], so it is cross-vendor.

**Levels of detail in the rays:**
- **Lloyd, Klehm, Stich (NVIDIA, 2020)** [32] [read]: two TLAS instances per object, one per
  candidate LOD. Instance masks have set bits in proportion to the crossing. Each ray carries a mask
  with one random bit, so 8 bits give 8 levels.
  - Costs: the TLAS doubles (+1.6 % render time with no band), selection 0.018 ms and TLAS build
    0.22 ms for 64×60 objects, up to 10 % divergence with wide bands.
  - Forge has five free mask bits [local].
- **Wind:** a BLAS holds the rest pose; Unreal culls WPO in RT by a radius
  (`r.RayTracing.StaticMeshes.WPO.CullingRadius`) [6] [read]. A swaying tree's shadow does not
  sway [inferred].
- **NVIDIA's coming foliage system** [31] [read]:
  - Partitioned top-level acceleration structures that instance and update large parts of a scene
    every frame. CD Projekt RED plans it for the Witcher 4.
  - A 5×5 km forest of about 60 million plants, with OMMs for distant levels [31] [search].
  - NVIDIA-only for now (DXR 2.0 is said to take it up [search]). Not for Forge's cross-vendor core.

**Grass and rays:**
- No source states outright that shipped games keep grass out of the BVH [search].
- Unreal's components carry "Visible in Ray Tracing" [5] [read].
- Tsushima judged full grass shadows too costly: impostors for the grass height plus screen-space
  shadows [19] [search: a secondary summary].
- Forge [inferred]: grass receives the sun's rays and is met by none. Every grass instance in the
  movers' TLAS (the outside scene's pool) makes every ray query it (#299's fact).

## 5. Dense grass

**Shipped and published methods:**
- **Ghost of Tsushima** (Wohllaib, GDC 2021) [19]:
  - Blades generated on the GPU, each with a procedural look and animation of its own [read: the
    session page].
  - From a secondary summary [search]:
    - each render tile has textures for height, material, and grass type and height;
    - a compute pass lays out the blades with frustum and distance culling;
    - instanced indexed draws with no vertex streams;
    - cubic Bézier blades, Voronoi clumps;
    - high and low LODs with a smooth transition;
    - normals tilted outwards for roundness and blended towards a clump normal with distance
      (against aliasing and gloss noise, notably in rain);
    - distant tiles use artist textures;
    - grass height copied to the CPU for stealth.
  - A forum comment says far blades were drawn twice as wide and half as many [search,
    unconfirmed].
  - GodotGrass (MIT) follows the talk's view-space widening of blades seen edge-on [read].
- **Black Ops 4's Blackout grass** (Pesce, first-hand) [21] [read]:
  - Blades are plain triangles, not alpha-cut cards, with the terrain's colour and normals; the
    grass occludes the terrain.
  - Normals shaped into a V across the blade. Translucency faked by flipping normals and boosting
    saturation away from the sun; without it the grass looked far too dark, even with GI. A height
    darkening ramp is the main occlusion.
  - Far away, blades shorten, then use fewer triangles, then are removed or sunk into the ground.
  - A NaN written to a vertex position kills its triangles, which is how the LOD drops them.
  - Tall grass removed by LOD could reveal hidden players, which drove volumetric experiments (not
    shipped).
- **Fortnite** [3] [read]: landscape grass spawns Nanite instances with built-in distance culling,
  real blade geometry with opaque materials, simple WPO.
- **Unreal's landscape grass** [42]:
  - The landscape material's grass output picks grass types by painted layer, with a density per
    type [read].
  - Start and end cull distances measured from the camera [search].
  - Built at run time for the components near the cameras [recalled].
- **AMD GPUOpen, mesh-shader grass** [33] [read]:
  - A quadratic Bézier (lean 0.3), 8 vertices and 6 triangles a blade, at most 32 blades a patch
    (DirectX's 256-vertex output cap), 128 threads.
  - Blade count falls from 32 to 2 with distance; the remaining blades widen by
    `maxBladeCount / bladeCountF`, and the last blade's width fades with the fraction.
  - Dark roots (`pow(..., 1.5)`), Perlin patches, normals 25 % towards up.
- **A Godot series** [34] [read]:
  - 5×5 m chunks of 10 000 blades of 9 triangles; the author puts about 10 M triangles a frame in
    the range of modern games.
  - The far LOD is one triangle a blade.
  - The fade is alpha-hash dithering (grass out over 10–20 m), with a ground "impostor" plane fading
    in over 5–10 m, under 2 ms.
- **Jahrmann & Wimmer 2017** (per-blade physics), the God of War wind field and Tsushima's wind
  talk: see [vegetation-materials.md](vegetation-materials.md) §2–§3.

**Placed from data, deterministically** [inferred unless marked]:
- Tsushima reads per-tile textures [19] [search]. Blackout stored per-tile bitmasks with run-length
  encoding, smaller than density maps except for very small objects [21] [read].
- For Forge (D-063): a density map per ground cell is cached with the region. A cell holds a
  density, a variety index, a height and a colour shift, derived once from the ground map's
  surfaces (D-061) and `forge_flora`'s fields (moisture, light under the canopy).
- A blade is then a function of (region seed, cell, index): the same blade every time, wherever
  the camera is. Nothing is generated by rules around the player (D-063's rule).

**LOD** [inferred unless marked]:
- Three levers:
  - fewer vertices a blade (Tsushima's two LODs, Godot's single triangle) [search, read];
  - fewer blades, widened to keep the cover (AMD [read], the Tsushima comment [search]). This is
    Pixar's live patent (§7);
  - shorter blades sinking into a ground layer that matches them (Blackout, the outside scene).
- Without the owner's word on the patent, use the first and third: counts fixed by the map, blade
  shape simplified with distance, height shrunk over the last metres into the ground's grass
  layer.

**Shading the owner asks for** [inferred, from the sources above]:
- A grass shading class (D-026) with:
  - lighter tips over dark depths: a height ramp of occlusion and colour (Blackout, AMD);
  - translucency towards the sun;
  - a rounded or V-shaped normal across the blade;
  - gloss stretched along the blade;
  - colour per clump;
  - a lean per region (a "combing", Pesce).
- With distance, the tip-to-root contrast and the per-blade normals converge to the clump's mean
  (Tsushima's specular rule). This removes the far contrast that doubled the outside scene's
  shimmer.

**Shimmer of thin blades** [inferred unless marked]:
- **The arithmetic:** at 1600 px across 90°, a blade w metres wide at d metres is 800·w/d px. A
  2 cm blade is one pixel at 16 m, a 1 cm blade at 8 m. Past that, the blade covers a pixel only on
  some of DLAA's 8 jitter phases, and the pixel flickers.
- **Humus' phone-wire AA** [35] [read]: draw thin geometry at least one pixel wide and fade it by
  the ratio. A blade at half a pixel is drawn one pixel wide at 0.5 coverage.
- **In a visibility buffer** the coverage is a keep/drop test by the gratings' spatio-temporal
  noise [local]. DLAA averages it over its 8 phases, as it does the gratings' bars.
- **Unreal 5.7's TSR** gained a "thin geometry detection" for foliage [4] [read]. DLAA's own
  handling of sub-pixel blades was not documented in anything read.
- **For cards, not blades:** Castaño's coverage-preserving alpha mips keep cards from thinning with
  distance (Golus 2017) [36] [read].

**Rays and interaction** [inferred]:
- Grass is met by no ray, as above.
- Its self-shadowing is the height ramp. The probes see the ground row tinted by its grass.
- Bending by walkers (a per-tile interaction texture, Tsushima and God of War) can come later. The
  outside scene did not ask for it.

**Cost numbers:**
- No first-party millisecond figure for grass was found.
- Assassin's Creed Shadows' 1.5 billion → 6.8 million triangles [22] [read] covers grass, bushes and
  trees together.
- The Godot series' 10 M triangles is a hobbyist's ballpark [34] [read].
- A Forge estimate [inferred], for 90° and 1.7 m eyes:
  - the view holds about 113 m² of ground within 12 m and 849 m² from 12 to 35 m;
  - at 400 blades/m² near and 150 beyond, that is about 172 000 blades;
  - at 6–12 triangles a blade, 1–2 M triangles: inside the AC Shadows and Godot figures.
  - To measure (§8).

## 6. An instance draw distance with a fade

#300's alternative: grass as instances drawn and traced within a distance, fading out.

- **Dither or scale** [read unless marked]:
  - **Dither:** Unreal's Dithered LOD Transition is a material flag, and an Epic answer warns of
    extra overdraw in the bands [42] [search]. Unity's HDRP uses a spatial hash [14]; Godot offers
    alpha or a dithered material [15]; the Godot grass series uses alpha hash [34].
  - **Scale:** the outside scene shrinks each patch over its last metres [local].
- **With the visibility buffer** [inferred]:
  - A dithered instance must go through the cut-out pipeline (discard), so it loses early depth
    while it fades. A shrinking one needs no discard.
  - For grass, shrinking into a matching ground layer looks like grass sinking into the turf
    (Blackout's choice) and costs nothing.
  - For trees, scale fades are visible (a tree growing). The dither is the tool for the ladder's
    crossings.
- **With DLAA** [inferred]:
  - Lagarde's warning is about noise that does not repeat [14] [read]. Forge's noise repeats with
    the 8 jitter phases [local], so a still view converges to the exact blend in 8 frames. In
    motion, DLAA sees a pattern and may keep some of it.
  - Brucks: the impostors' dithering needs temporal AA [10] [read].
  - To measure on the turn and the dolly (§8).
- **With the TLAS** [inferred]:
  - "Traced only within the distance" means changing the static TLAS as the camera moves (an
    update per frame with new masks, or rebuilds), or keeping the faded set in the movers' TLAS,
    which every ray then queries (#299).
  - Grass needs neither if it is never traced. Trees need neither if the rays see a fixed
    representation (§8).
- **Verdict on #300's alternative** [inferred]:
  - A draw distance with a scale fade is the right stopgap for grass patches. Its fade is the one
    the GPU pass should keep.
  - Patches as instances repeat (one 1 m patch per square) and cost a CPU pool. Moving them to the
    GPU pass removes both, and the "traced within" part should be dropped.

## 7. Patents

Found with WebSearch and Google Patents on 2026-10-19. Statuses and expiries are Google Patents'
own estimates. A record of what was met, not a freedom-to-operate review. "Claims read" means the
claim text was reached on the page.

| Technique | Patent | Holder | Juris. | Status | Expiry (est.) | What the claims cover | Alternative |
|---|---|---|---|---|---|---|---|
| Leaves or blades pruned and the rest modified to stand in (fewer, larger) | US 7,973,789 B2 [P11] (US 2006/0274076 A1) | Pixar | US | active | ≈ 2028-11-10 | claim 1: select a subset of a model's 3D elements, modify them, a modified element represents an unselected one, render; claim 20 ties the subset to a level of detail; dependents: area, screen size, contrast (claims read) | counts fixed by the map; simplify each element's shape; hand over to a different representation (a lump shell, the ground's grass layer); or wait for 2028. **Ask the owner** |
| Stochastic level of detail (per-sample choice between representations) | US 6,300,956 B1, RE 42,287 E1, EP 1 064 619 B1 [P4] | Pixar | US, EP | expired (ceased) | 2018-03-17 | claim 1: several representations, each sample given one, contributions combined (claim read) | free: the dithered crossing rests here |
| Dissolve by a dither pattern | US 5,771,033 A [P5] | Microsoft | US | expired | 2016-05-24 | dissolving an image through a dither | free |
| Texture LOD dithering | US 6,191,793 B1 [P6] | Real 3D (Intel) | US | expired | 2018-04-01 | dithered offsets on texture LOD | free |
| Hashed alpha testing | US 10,417,813 B2 [P3] | NVIDIA | US | active | ≈ 2037-11-10 | stable hash of object-space coordinates scaled to pixel size as the alpha threshold (abstract read) | screen-space noise repeating with the jitter (Forge's gratings), coverage-preserving mips |
| Opacity micromaps in hardware traversal | US 12,475,631 B2 [P1] (US 2023/0084570 A1) | NVIDIA | US | active | ≈ 2043-05-09 | ray tracing hardware indexing a visibility mask by barycentrics to ignore or commit a hit; 4-state remapping (claims read) | use OMMs only through the vendor's driver; do not emulate them in software in the candidate loop; the per-geometry opaque split |
| Micromaps as a tree | US 12,541,921 B2 [P2] | ARM | US | active | ≈ 2044-06-11 | traversing a tree form of a micromap (abstract read) | not needed |
| LOD in rays by comparison masks | US 11,989,817 B2 [P7] (US 2022/0012934 A1) | Intel | US | active | ≈ 2042-01-11 | an instance comparison mask (N-bit value plus comparison type) compared with a ray value to pick an LOD (the numbered examples read; formal claims not reached) | Lloyd's AND masks (its own background) or choosing the mask per ray in the shader |
| LOD node in a BVH chosen by ray distance | US 10,311,629 B2 [P8] | Intel | US | active | ≈ 2036-01-22 | a BVH node with distances and a bias picking the level (abstract read) | the mask chosen before tracing, by the ray origin's distance from the camera |
| Ray-tracing LOD with residency | US 12,198,248 B2 (US 2024/0070960 A1) [P9] | Microsoft | US | active | ≈ 2043-01-31 | LOD per sub-tree from a residency map (abstract read) | not needed |
| Ray hits on two LOD representations | US 7,126,605 B1 [P10] | Munshi, Wood-Patrick (now Cloudrock) | US | expired | 2024-02-27 | intersections with two levels of an object (claim 1 read) | free |
| Simplification by image loss (differentiable rendering) | US 11,450,077 B2, US 11,615,602 B2 [P12] | NVIDIA | US (DE filing) | active | ≈ 2041-03-08 | render a simplified and a reference model, adjust parameters (with a displacement map) to reduce image differences; claim 12 a differentiable renderer (claims read) | geometric fitting: billboard clouds by plane fitting (Décoret 2003), lump shells from the graph |
| Dynamic 2D impostors (crowds) | US 7,019,742 B2 (US 2005/0110789 A1) [P13] | Microsoft | US | expired (fees) | 2024-09-29 | impostors from a shared projection point (claims not read) | free |
| Impostor generation with MIP anti-aliasing | US 2005/0017982 A1 [P14] | SDS International | US | abandoned 2006 | — | — | free |
| Texel imagery (point clouds as quads and splats; billboard clouds in its description) | US 7,940,279 B2 [P15] | Utah State University | US | active | ≈ 2030-02-19 | claims not read; about lidar point-cloud octrees | not Forge's method |
| Flora generated by rules around the avatar | US 8,115,765 B2 [P16] | Sony Online (now Daybreak) | US, JP | active | ≈ 2028-01-07 | claim 38 (independent): hierarchical layered rules (boundaries, filters, affectors) with blending, text rules, rules added or removed, flora generated in a user-defined area around an avatar; claim 39: density and new locations as it moves (claims read) | placement as cached data per region, drawn within a distance; the grass pass waits for **the owner's word** (D-063 point 1) |
| Automatic grass placement in a field | JP 3,943,097 B2 (JP 2005-293470 A) [P17] | Konami | JP | expired | 2024 | grass placed to a density at field generation, smaller at the edges | free |
| Texture fading between LODs over time | US 8,654,124 B2 [P18] | Google | US | expired (fees) | — | blending terrain textures over time between LODs | free |

**Looked for and not found** [search]: patents on octahedral or hemi-octahedral impostors, on
billboard clouds as a method, on GPU-generated grass blades, on vertex wind for vegetation (only an
unread Chinese application, CN 103646413 A, on OSG trees), on voxel aggregates of foliage (no Epic
patent surfaced), on ray-tracing proxies for crowns, and on the BLAS split by opacity (a core API
feature). Their absence from search results is not proof.

**Met in passing:** US 8,190,360 B2 ("visual effects on grass") is a lawn-printing machine, not
graphics [P19]. Imagination's probe patents and Square Enix's are covered in
[lighting-patents.md](lighting-patents.md); crown proxies change nothing there.

## 8. A proposal for Forge

*Opinion, shaped by the engine's facts in §1 and the owner's rules.*

### 8.1 The ladder per tree, cooked from `forge_flora`'s graph

For each species and age class (D-063 point 3), into D-053's cache [inferred]:

| Level | What | Built from the graph | Under the cluster DAG |
|---|---|---|---|
| L0, near | wood ≥ 5 cm (≥ 2 cm close), cards per leaf cluster or lump, lump cores | today's `grown_mesh` | the wood and cores simplify; the cards stay (until #303's options) |
| L1, mid | main limbs and the crown as closed bumped shells, one per lump, with the canopy texture and a cut-out only at the silhouette | today's stand-ins and `grown_proxy`'s shells | closed: simplifies to a few hundred triangles |
| L2, far | hemi-octahedral impostor, 8×8 to 12×12 frames, base colour and alpha, normal and depth; three frames blended, one depth offset | baked from L0 by Forge's own renderer, neutral light | a quad, its own shading class |
| R, rays | wood and each lump's shell at 0.8 radius (#297); later, an analytic crown per lump | `grown_proxy` | its own BLAS |

- **One row block per species** [inferred]. The levels share sections (wood 0, cores 1, cards 2,
  shells 3, impostor 4), so `inst.material + section` stays right whatever level a cluster comes
  from.
- **The impostor is a shading class** (D-026) that reads its atlas. Its normal and depth let the
  sun's ray and the probes light it like the others.

### 8.2 Choosing the level and hiding the crossing

- **Per instance, in the cull, by projected size in output pixels** (like the DAG's error), not by
  a static split [inferred].
  - A ladder record per mesh: the next mesh, the size at which to cross, the band's width.
  - A per-instance offset (hashed from the instance id, ±15 %) so the crossings form no ring.
  - Hysteresis on the way back.
- **Starting thresholds, to tune by eye and numbers** [inferred], at 1600 px across 90°:
  - L0 → L1 when a lump is about 48 px across: 25–40 m for the city's trees (the outside scene's
    25 m);
  - L1 → L2 when the crown is about 64 px: a 10 m crown at about 125 m (D-013 said 150 m);
  - impostor frames of at least twice the crown's size at the crossing: 12×12 frames of 128 px
    make a 1536² atlas.
- **The crossing** [inferred]: both levels are drawn in the band; each pixel keeps one.
  - L0 discards where `noise < t`, the coarser level where `noise ≥ t`, `t` the crossing's
    fraction, `noise = spatio_temporal_noise(pixel, noise_frame)`. Complementary, so no holes and
    no doubles.
  - The crossing value goes from the cull to `cutout_kept` (a per-frame byte per visible instance).
  - Only crossing trees pay: two levels, and the cut-out pipeline for the coarser one.
  - The noise repeats with DLAA's 8 phases, so a still view converges in 8 frames.
  - If motion shows the dither, try a time-driven fade (Unity's "animate cross-fading": once a
    tree crosses, it fades over a fixed time) [14] [read].
- **Inside each level the DAG keeps working**: the wood, the shells and (with #303's options) the
  cards simplify by error as any mesh.
- **Later**, once #303's authored parents exist, the L0 → L1 crossing can move into the DAG, group
  by group (a branch's cards to its lump), with the same complementary dither on the group's band.
  Each lump would then cross on its own, as Nanite's voxels do.

### 8.3 What each ray sees at each level

| Level drawn | Sun and lamp shadow rays | Probe rays | Mirror rays |
|---|---|---|---|
| L0 cards | cards (masked geometry only; OMMs where allowed) and wood, when the ray starts within the card radius | proxy R (#297) | proxy R |
| L1 shells | proxy R, beyond the card radius | proxy R | proxy R |
| L2 impostor | proxy R | proxy R | proxy R |
| grass | none (grass receives rays, casts none) | none | none |

- **The rays see a fixed representation per tree** [inferred]. The static TLAS holds two instances
  per tree:
  - the cards mesh with a new `RAY_SHADOW_NEAR` bit;
  - proxy R with `RAY_SHADOW_FAR | RAY_PROBE | RAY_MIRROR`.
- **A shadow ray picks its bit by its origin's distance from the camera.** A tree's shadow falls
  within a tree's height of it, so this follows the drawn level.
  - Over the band the choice uses the pixel's same noise value, so shadows cross with the trees.
  - Nothing in the TLAS changes as the camera moves: no updates, no rebuilds, no movers.
  - Lloyd's AND masks in a simpler form; Intel's comparison masks and BVH distance nodes are not
    used (§7).
- **The probes keep #297's proxy.** Measure its over-occlusion against the cards' (the probes'
  irradiance under a crown); if it is too dark, try the stochastic transmittance, then the analytic
  crown.
- **Cost:** two TLAS records per tree. At a million trees that is about 128 MB of instance records,
  plus the TLAS nodes; to measure against D-029's 278 MiB for the city's million instances
  [inferred].

### 8.4 Grass near the camera

- **Data:** the density map per region (§5), cached; blades a pure function of (region seed,
  cell, index) [inferred].
- **The pass** [inferred]:
  - A compute pass culls the tiles within about 35 m (frustum, distance, HZB). It writes per-blade
    parameters (16 bytes: root, lean, height, width, variety) for the visible blades into a
    transient buffer. About 172 000 blades make 2.75 MB.
  - A mesh-shader pass draws the blades as patches into the visibility buffer: opaque, no alpha.
    A patch keeps within the id's 7-bit triangle field and the 256 output vertices: 12 near blades
    of 10 triangles (12 vertices each), or 20 mid blades of 6 triangles (8 vertices each).
  - The resolve rebuilds a blade's triangle from the same parameters with the same function.
- **Bit-exact rebuilding** [local: D-016's note on #281, `forge-gpu/src/shader.rs`,
  `shaders/exact.slang`]: the blade's positions must come out identical in the mesh pass and the
  resolve. Compile the function in both with `-fp-mode precise` (`PRECISE_ENTRIES`) and mix with
  `lerp_exact`, or fused multiply-adds will part the two by an ulp (#277, #281).
- **The fallback rasteriser** (D-003) needs the same pass as an indirect draw, for the A/B harness
  (0 pixels).
- **LOD** [inferred]:
  - 6 → 4 → 2 vertices along each edge of a blade (10, 6 and 2 triangles) with distance, the
    blade count fixed by the map;
  - beyond about 30 m the blades shrink in height into the ground's grass layer over 5 m;
  - the layer is coloured with the blades' far mean.
  - No pruning with widening unless the owner decides on Pixar's patent.
- **Shading:** the grass class of §5. Minimum one output pixel of width, with coverage by the
  gratings' noise. Contrast and normals converge with distance.
- **Rays:** none. The pass's instances never enter a TLAS; probes see the ground row tinted.
- **Wind:** D-013's shared field, the same that later bends the trees.
- **The stopgap until then:** #300's draw distance with a scale fade for the outside scene's
  patches, never in a TLAS.

### 8.5 The engine changes, as issues

1. **Split each BLAS by opacity** (opaque rows / masked rows as two geometries; no
   `FORCE_NO_OPAQUE`). Small, cross-vendor. Measure the sun's and lamps' rays (`shading/*`) and the
   mirrors on #302's town view before anything else: it may take part of the stand-ins' gain with
   the cards kept.
2. **The ladder record and the per-instance crossing** in the cull and `cutout_kept` (#302).
3. **Two TLAS instances per tree and the shadow-ray bit by origin distance** (#302).
4. **The impostor baker and shading class** (D-013's L2), into D-053's cache.
5. **The grass pass** (#300), with its fallback.
6. **#303's per-mesh options**: open borders free, `Prune`, authored parents, an appearance error.
7. **Optional, on the owner's word:** `VK_KHR_opacity_micromap` for the card geometry on devices
   that expose it, with Forge's own baker. The OMM SDK's licence was not read.

### 8.6 Numbers to measure, and the demo

- **Cost** (best of three, `FORGE_ASYNC=0` for added passes):
  - the frame, `gi/probe rays`, `shading/*`, the raster passes and the new zones (`foliage/ladder`,
    `grass/cull`, `grass/raster`);
  - all cards against the ladder against all stand-ins, on #302's town view;
  - drawn triangles and crossing trees per frame.
- **Shimmer:** % of unstable pixels in a DLAA turn against `--ssaa`.
  - Target: the forest at or under the cards' 4.5 %.
  - Grass: under twice the bare ground's (the outside scene doubled it).
  - Each build against its own SSAA run (`imgdiff --then`).
- **Pops:**
  - a still camera with `--fixed-step --capture-every 10` under DLAA (the 0.05 % baseline);
  - a `--dolly` through the crossing bands: changed pixels per frame against a run with the
    crossings frozen.
- **Grass:** blades drawn, ground visible (% of grass-covered pixels showing the ground row at 2 m
  and 10 m), ms per pass.
- **Memory:** impostor atlases per species and age class (Fortnite's 6.8 MB a tree as the upper
  mark), BLASs, the TLAS with two records per tree.
- **The demo:** `forge_flora`'s hillside (about 266 000 plants per km² as placed today) in the
  village map or a `--forest` view of city-plan.
  - A walk from a meadow with grass to the knee into a holm-oak wood, then out to a view over the
    wooded valley.
  - A debug view colours each tree by its level and crossing.

## 9. Questions for the owner

1. **The crossing:** may the ladder's crossings be a dither that DLAA resolves (both levels drawn
   in a band), or should the near → mid crossing wait for the DAG's per-lump version (§8.2, later)?
2. **Pixar's US 7,973,789 B2** (to about 2028-11-10): may Forge thin leaves or blades and enlarge
   the rest (as Nanite's Preserve Area, SpeedTree and AMD's grass do), or keep counts fixed and
   hand over to other representations until 2028?
3. **The grass pass and Sony Online's US 8,115,765 B2** (to about 2028-01-07): the blades would be
   a cached function of each region's map, drawn within a distance of the camera, with no rules
   evaluated around the player. Is that inside D-063's rule, or does it wait?
4. **Opacity micromaps:** a `VK_KHR` extension the RTX 5070 Ti exposes but RDNA 4's status is
   unknown. Allowed as an optional speed-up with the any-hit loop as the fallback, or not before
   AMD has it?
5. **Grass met by no ray:** no grass shadows on other things, and probes that see the ground's
   tinted row rather than blades. Acceptable?
6. **Grass bent by walkers:** now, or after the carpet looks right?
7. **Impostor memory:** what budget per species and age class (Fortnite's 6.8 MB a tree, or a
   quarter of that at 1024² and 512²)?
8. **The far canopy** (D-013's band beyond 600 m): needed for the village maps, or only for the
   island and the planet later?

## 10. Checked and left out

- **Engines with no public vegetation-rendering source found** [search]:
  - Red Dead Redemption 2, Far Cry 5/6 (only the terrain talk), Call of Duty Warzone (only
    Pesce's Blackout posts);
  - Microsoft Flight Simulator 2020/2024 (forum observations only: ground textures first, trees
    filling in near the aircraft), Kingdom Come: Deliverance 2 (benchmark sites only);
  - Death Stranding 2 (an interview on terrain LOD only), Enshrouded (its GI talk only), Battlefield
    6 (an artist's portfolio only), Zelda BotW/TotK (forum guesses only).
  - The Witcher 3: SpeedTree, about 200 plant varieties and 40 grass varieties (vendor
    write-ups), three LODs plus an optional billboard for meshes in general [25] [search].
  - Battlefield 3's terrain decoration came from rules and masks at run time [26] [search].
- **Talks not read:**
  - GDC Vault videos are members-only, and YouTube was not watched. Ghost of Tsushima's blade
    counts and vertex numbers are therefore not given.
  - Slides offered only as downloads (Remedy's `.ppsx`, Guerrilla's `.pptx`) were not downloaded,
    by the rule.
  - The AC Shadows RT PDF is over 10 MB; Mittring's CryEngine 2 PDF did not convert to text.
- **Volumetric billboards, Bruneton–Neyret's far forests** (in vegetation-materials.md):
  unchanged. The horizon band waits for question 8.
- **SGGX voxels, hybrid mesh-volume LODs, neural prefiltering:** the direction of Nanite's voxels.
  They belong to a later aggregate level behind D-013's interface, not to #302.
- **Work graphs with mesh nodes** (AMD's procedural grass and trees, 79 710 instances in 3.74 ms on
  an RX 7900 XTX) [33] [search]: Direct3D 12 only; Vulkan has only a provisional AMD extension
  [recalled].
- **RTX Mega Geometry, cluster and partitioned acceleration structures:** NVIDIA-only extensions
  (present on the 5070 Ti [local]); outside the cross-vendor rule until a KHR form exists.
- **Image-loss fitting of cards** (Hasselgren 2021): patented (§7); plane fitting does the job.
- **The DLSS programming guide's word on dithered transparency:** the guide is a PDF and was not
  read; DLAA's behaviour on the crossing dither is to be measured.

## Verification notes

- **Read on 2026-10-19:**
  - Epic's Nanite Foliage (5.8), Nanite, Lumen technical details, hardware RT tips, the ray
    tracing proxy settings API and the landscape grass quick start; the Epic forum thread of July
    2026.
  - Tom Looman on 5.7. The Fortnite Nanite post and Brucks' impostor post, through the reader proxy
    `r.jina.ai` (the hosts refuse fetches).
  - The SpeedTree runtime LOD and glossary pages; Unity's LOD transitions and Lagarde's 2018 post;
    Godot's visibility ranges.
  - Guerrilla's McLaren page; the Tsushima and Samurai Landscapes session pages; Pesce's two posts;
    PCGH's AC Shadows report; the Advances 2025 and 2026 course pages.
  - Remedy's GDC 2024 page and its skinning session page; Massive's BVH session page.
  - NVIDIA's Indiana Jones OMM post, its 2026-03-10 RTX post and Lloyd et al. 2020; Microsoft's
    OMM post; the Vulkan registry page; the NVIDIA-RTX/OMM repository page.
  - GPUOpen's grass post, the Godot grass series part 4, Humus' phone-wire AA, Golus 2017 (through
    the proxy), Gasch et al. 2018, the meshoptimizer README, GodotGrass and the two impostor
    repositories.
  - Google Patents pages [P1]–[P19], with the claims reached for P1, P4, P7 (examples), P10, P11,
    P12 and P16.
- **[local]:**
  - `docs/DECISIONS.md` (D-008, D-013, D-021, D-026, D-029 with #301's note from the main
    checkout's working tree, D-036, D-063);
  - `demos/city-plan/src/lib.rs` (`grown_mesh`, `grown_proxy`, `crown_lumps`, `leaf_cards`,
    `lump_of`, from the main checkout, which holds #297), `crates/forge-geom/src/lod.rs`,
    `shaders/meshlet.slang`, `shaders/noise.slang`, `crates/forge-gpu/src/accel.rs`;
  - the vendored `meshoptimizer.h` (0.25) in `meshopt` 0.6.2;
  - `vulkaninfo` on this machine (RTX 5070 Ti, driver 617.42, API 1.4.351; AMD Radeon iGPU,
    driver 2.0.353).
- **Search-recorded only:** the Witcher 4 demo coverage, SpeedTree prices, CryEngine's dissolve,
  Sanders' HZD talk (an AI summary), the Tsushima secondary summary and the forum comment, the
  Vulkan 1.4.351 news, the AMD RDNA 4 OMM status, the GDC 2026 forest figures, Cook 2007,
  Hasselgren 2021, Loubet & Neyret 2017, Weier 2023, Unreal's grass cull distances, the engines of
  §10.
- **Numbers that are Forge's estimates, not measurements:** the thresholds of §8.2, the grass
  counts and triangles of §5 and §8.4, the blade-pixel arithmetic, and the TLAS memory of §8.3.

## Sources

1. Epic Games, "Nanite Foliage" (Unreal Engine 5.8 documentation) —
   <https://dev.epicgames.com/documentation/unreal-engine/nanite-foliage> [read]
2. Epic Games, "Nanite Virtualized Geometry" —
   <https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-virtualized-geometry-in-unreal-engine> [read]
3. Epic Games, "Bringing Nanite to Fortnite Battle Royale in Chapter 4" (2023) —
   <https://www.unrealengine.com/tech-blog/bringing-nanite-to-fortnite-battle-royale-in-chapter-4> [read via r.jina.ai]
4. T. Looman, "Unreal Engine 5.7 performance highlights" —
   <https://tomlooman.com/unreal-engine-5-7-performance-highlights/> [read]
5. Epic Games, "Lumen Technical Details" —
   <https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine> [read]
6. Epic Games, "Hardware Ray Tracing Tips and Tricks" —
   <https://dev.epicgames.com/documentation/en-us/unreal-engine/hardware-ray-tracing-tips-and-tricks-in-unreal-engine> [read]
7. Epic Games, `unreal.MeshRayTracingProxySettings` (Python API 5.8) —
   <https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/MeshRayTracingProxySettings> [read]
8. Epic forums, "Raytracing Fallback Mesh for Nanite Foliage" (July 2026) —
   <https://forums.unrealengine.com/t/raytracing-fallback-mesh-for-nanite-foliage/2743156> [read]; "Ray tracing proxy problem for Foliage" —
   <https://forums.unrealengine.com/t/ray-tracing-proxy-problem-for-foliage/2746871> [read]
9. The Witcher 4 Unreal Engine 5.6 demo (State of Unreal, June 2025), press coverage —
   <https://hothardware.com/news/witcher-4-unreal-engine-5-6-demo> ;
   <https://www.gamestar.de/artikel/unreal-engine-5-6-state-of-recap,3434063.html> [search]
10. R. Brucks, "Octahedral Impostors" (2018) — <https://shaderbits.com/blog/octahedral-impostors> [read via r.jina.ai]
11. SIsilicon, Godot Octahedral Impostors (MIT) — <https://github.com/SIsilicon/Godot-Octahedral-Impostors> [read];
    R. Brucks, ImpostorBaker (no licence shown) — <https://github.com/ictusbrucks/ImpostorBaker> [read]
12. Unity, SpeedTree Runtime SDK, "Level of detail" —
    <https://docs.unity3d.com/speedtree-runtime-sdk/manual/level-of-detail.html> [read];
    SpeedTree Modeler glossary — <https://docs.unity3d.com/speedtree-modeler/manual/glossary.html> [read]
13. Unity support, "How can I purchase a SpeedTree license" —
    <https://support.unity.com/hc/en-us/articles/30682946395156-How-can-I-purchase-a-SpeedTree-license> [search]
14. Unity Manual, "LOD transitions" — <https://docs.unity3d.com/Manual/lod/lod-transitions-lod-group.html> [read];
    S. Lagarde, "Blue noise crossfade / dither / lod" (2018-09-26) —
    <https://discussions.unity.com/t/blue-noise-crossfade-dither-lod/706936> [read]
15. Godot Engine, "Visibility ranges (HLOD)" —
    <https://docs.godotengine.org/en/stable/tutorials/3d/visibility_ranges.html> [read]
16. CRYENGINE 3 documentation (`e_Dissolve`, `e_VegetationSpritesDistanceRatio`) —
    <https://cryengine.com/docs/static/engines/cryengine-3/categories/9895942/pages/9219666> [search];
    M. Mittring, "Finding Next Gen — CryEngine 2", SIGGRAPH 2007 —
    <https://www.advances.realtimerendering.com/s2007/Mittring-Finding_NextGen_CryEngine2(Siggraph07).pdf> [search]
17. G. Sanders, "Between Tech and Art: The Vegetation of Horizon Zero Dawn", GDC 2018 —
    <https://gdcvault.com/play/1025066/Between-Tech-and-Art-The> ; summary <https://videohighlight.com/v/wavnKZNSYqU> [search]
18. J. McLaren, "Adventures with Deferred Texturing in Horizon Forbidden West", GDC 2022 —
    <https://www.guerrilla-games.com/read/adventures-with-deferred-texturing-in-horizon-forbidden-west> [read]
19. E. Wohllaib, "Procedural Grass in Ghost of Tsushima", GDC 2021 —
    <https://gdcvault.com/play/1027214/Advanced-Graphics-Summit-Procedural-Grass> [read: description];
    secondary summary — <https://gist.ly/youtube-summarizer/procedural-grass-systems-in-ghost-of-tsushima-achieving-art-direction> [search];
    2Retr0, GodotGrass (MIT) — <https://github.com/2Retr0/GodotGrass> [read]
20. M. Pohlmann, "Samurai Landscapes: Building and Rendering Tsushima Island on PS4", GDC 2021 —
    <https://gdcvault.com/play/1027352/Samurai-Landscapes-Building-and-Rendering> [read: description]
21. A. Pesce, "Vegetation system" and "Vegetation, part 2" —
    <https://www.c0de517e.com/015_vegetation_system.htm> ; <https://c0de517e.com/017_vegetation_part2.htm> [read]
22. PCGH, "Anvil Engine & Assassin's Creed Shadows" (GDC 2025 report) —
    <https://www.pcgameshardware.de/Assassins-Creed-Shadows-Spiel-74683/Specials/GDC-2025-Anvil-Engine-1468846/> [read];
    Advances in Real-Time Rendering 2025 — <https://advances.realtimerendering.com/s2025/index.html> [read];
    2026 — <https://advances.realtimerendering.com/s2026/index.html> [read]
23. Remedy, "Explore Remedy's GDC 2024 talks" —
    <https://remedygames.com/article/explore-remedys-gdc2024-talks-on-creating-alan-wake-2> [read];
    K. Kandar, "Large Scale GPU-Based Skinning for Vegetation in Alan Wake 2", GDC 2024 —
    <https://gdcvault.com/play/1034310/Large-Scale-GPU-Based-Skinning> [read: description]
24. O. Koshlo, "Ray Tracing in Snowdrop: Scene Representation and Custom BVH", GDC 2024 —
    <https://gdcvault.com/play/1034277/Ray-Tracing-in-Snowdrop-Scene> [read: description];
    PCGH on Avatar's tech —
    <https://www.pcgameshardware.de/Avatar-Frontiers-of-Pandora-Spiel-73347/Specials/Avatar-Frontiers-of-Pandora-Snowdrop-Tech-Benchmarks-Test-1435474/2/> [search]
25. M. Gollent, "Landscape Creation and Rendering in REDengine 3", GDC 2014 —
    <https://www.gdcvault.com/play/1020394/Landscape-Creation-and-Rendering-in> [search]
26. DICE, "Terrain in Battlefield 3", GDC 2012 —
    <https://media.contentapi.ea.com/content/dam/eacom/frostbite/files/gdc12-terrain-in-battlefield3.pdf> [search]
27. NVIDIA, "Path Tracing Optimizations in Indiana Jones: Opacity MicroMaps and Compaction of
    Dynamic BLASs" — <https://developer.nvidia.com/blog/path-tracing-optimizations-in-indiana-jones-opacity-micromaps-and-compaction-of-dynamic-blass> [read]
28. Microsoft, "D3D12 Opacity Micromaps" (2026-02-26) — <https://devblogs.microsoft.com/directx/d3d12-opacity-micromaps/> [read]
29. Khronos, `VK_EXT_opacity_micromap` —
    <https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_opacity_micromap.html> [read];
    Phoronix, "Vulkan 1.4.351" — <https://www.phoronix.com/news/Vulkan-1.4.351-Released> [search]
30. NVIDIA-RTX, Opacity Micro-Map SDK — <https://github.com/NVIDIA-RTX/OMM> [read]
31. NVIDIA, "NVIDIA RTX Innovations Are Powering the Next Era of Game Development" (2026-03-10) —
    <https://developer.nvidia.com/blog/nvidia-rtx-innovations-are-powering-the-next-era-of-game-development> [read];
    Igor's Lab on GDC 2026 —
    <https://www.igorslab.de/en/the-geforce-platform-in-transition-ai-reconstruction-ray-tracing-and-new-rendering-structures-as-the-basis-for-the-next-generation-of-games-gdc-2026/5/> [read]; its page 4 [search]
32. B. Lloyd, O. Klehm, M. Stich, "Implementing Stochastic Levels of Detail with Microsoft DirectX
    Raytracing" (2020) — <https://developer.nvidia.com/blog/implementing-stochastic-lod-with-microsoft-dxr> [read]
33. AMD GPUOpen, "Procedural grass rendering" —
    <https://gpuopen.com/learn/mesh_shaders/mesh_shaders-procedural_grass_rendering/> [read];
    "Procedural generation" (work graphs) and Kuth et al., TOG 2024 —
    <https://gpuopen.com/learn/work_graphs_mesh_nodes/work_graphs_mesh_nodes-procedural_generation/> [search]
34. hexaquo, "Grass rendering series part 4: level of detail tricks" —
    <https://hexaquo.at/pages/grass-rendering-series-part-4-level-of-detail-tricks-for-infinite-plains-of-grass-in-godot/> [read]
35. Humus, "Phone-wire AA" (2012) — <https://www.humus.name/index.php?page=3D&ID=89> [read]
36. B. Golus, "Anti-aliased Alpha Test: The Esoteric Alpha To Coverage" (2017) —
    <https://bgolus.medium.com/anti-aliased-alpha-test-the-esoteric-alpha-to-coverage-8b177335ae4f> [read via r.jina.ai]
37. A. Kapoulkine, meshoptimizer — <https://github.com/zeux/meshoptimizer> [read]; `meshoptimizer.h`
    0.25 vendored in `meshopt` 0.6.2 [source]
38. C. Gasch, I. Remolar, M. Chover, C. Rebollo, "Viewpoint-Driven Simplification of Plant and Tree
    Foliage", *Entropy* 20(4), 2018 — <https://pmc.ncbi.nlm.nih.gov/articles/PMC7512728/> [read]
39. R. L. Cook, J. Halstead, M. Planck, D. Ryu, "Stochastic Simplification of Aggregate Detail",
    SIGGRAPH 2007 — <https://history.siggraph.org/?p=115388> [search]
40. J. Hasselgren, J. Munkberg, J. Lehtinen, M. Aittala, S. Laine, "Appearance-Driven Automatic 3D
    Model Simplification", EGSR 2021 — <https://arxiv.org/abs/2104.03989> [search]
41. G. Loubet, F. Neyret, "Hybrid Mesh-volume LoDs for All-scale Pre-filtering of Complex 3D
    Assets", *CGF* 36(2), 2017 — <https://diglib.eg.org/handle/10.1111/cgf13138> [search];
    P. Weier et al., "Neural Prefiltering for Correlation-Aware Levels of Detail", SIGGRAPH 2023 —
    <https://graphics.cg.uni-saarland.de/publications/weier-2023-neural-lod.html> [search];
    E. Heitz, J. Dupuy, C. Crassin, C. Dachsbacher, "The SGGX Microflake Distribution", SIGGRAPH 2015 [recalled]
42. Epic Games, "Grass Quick Start" —
    <https://dev.epicgames.com/documentation/en-us/unreal-engine/grass-quick-start-in-unreal-engine> [read];
    "Open World Tools Tips" (cull distances) —
    <https://dev.epicgames.com/documentation/unreal-engine/open-world-tools-tips-in-unreal-engine> [search];
    Unreal answers on dithered LOD transitions (2015–2017) — <https://answers.unrealengine.com/questions/707820/view.html> [search]

**Patents** (Google Patents pages, read on 2026-10-19):

- P1. US 12,475,631 B2 — <https://patents.google.com/patent/US12475631B2/en>
- P2. US 12,541,921 B2 — <https://patents.google.com/patent/US12541921B2/en>
- P3. US 10,417,813 B2 — <https://patents.google.com/patent/US10417813B2/en>
- P4. US 6,300,956 B1 — <https://patents.google.com/patent/US6300956B1/en>
- P5. US 5,771,033 A — <https://patents.google.com/patent/US5771033A/en>
- P6. US 6,191,793 B1 — <https://patents.google.com/patent/US6191793B1/en>
- P7. US 11,989,817 B2 — <https://patents.google.com/patent/US11989817B2/en>
- P8. US 10,311,629 B2 — <https://patents.google.com/patent/US10311629B2/en>
- P9. US 2024/0070960 A1 (US 12,198,248 B2) — <https://patents.google.com/patent/US20240070960A1/en>
- P10. US 7,126,605 B1 — <https://patents.google.com/patent/US7126605B1/en>
- P11. US 7,973,789 B2 — <https://patents.google.com/patent/US7973789B2/en>
- P12. US 11,450,077 B2 — <https://patents.google.com/patent/US11450077B2/en>
- P13. US 2005/0110789 A1 (US 7,019,742 B2) — <https://patents.google.com/patent/US20050110789>
- P14. US 2005/0017982 A1 — <https://patents.google.com/patent/US20050017982>
- P15. US 7,940,279 B2 — <https://patents.google.com/patent/US7940279>
- P16. US 8,115,765 B2 — <https://patents.google.com/patent/US8115765B2/en>
- P17. JP 2005-293470 A (JP 3,943,097 B2) — <https://patents.google.com/patent/JP2005293470A/en>
- P18. US 8,654,124 B2 — <https://patents.google.com/patent/US8654124>
- P19. US 8,190,360 B2 — <https://patents.google.com/patent/US8190360>
