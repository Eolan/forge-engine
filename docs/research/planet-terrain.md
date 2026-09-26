# Research — Planet terrain: the cube sphere from orbit to the ground

> Companion to `terrain-genesis.md` (the genesis, whose last paragraph says the planet runs the
> same stages on the cube sphere's coarse graph with tiles amplified at streaming time), to
> `large-worlds.md` §1, §5 and §8 (coordinates, World Partition, D-014's far-field choice), to
> `memory-streaming.md` §4 (the page pool) and to `planet-environment.md` (climate, weather,
> clouds). Written 2026-09-26 for the planet variant of Phase 2's `island` demo
> (`docs/ROADMAP.md`: "orbit-to-ground on the planet variant"), against D-037 🟡 (the
> equi-angular cube sphere, the `u64` cell ids, the clipmap of cells) as `crates/forge-world`
> implements it. Every citation was checked that day against a reachable page or, where the
> network proxy refused the host, against the search engine's record of it; the distinction is
> kept per entry under [Verification notes](#verification-notes) for issue #99, and what could
> not be found is under [Checked and left out](#checked-and-left-out).

The question is how a whole planet's terrain reaches the screen from orbit down to the 4 m field
the island already has, without the three things the owner sees first (a pop when a tile swaps,
a shimmer at the horizon, a crack between levels), and what it costs: which partition of the
sphere, which LOD scheme on it, how the horizon and the precision are handled, how tiles are
generated on demand and streamed through the job system and the page pool, what the atmosphere
pass already gives, and what to measure. The short answer: the partition is settled and in the
code (D-037's equi-angular cube sphere with S2-style ids); the LOD literature's three answers for
a heightfield (clipmaps on the sphere, CDLOD's morph, concurrent binary trees) are ways of
avoiding a mesh hierarchy Forge already has, so the planet's tiles are cluster-DAG props like the
city's ground, one per cell of D-037's clipmap, cooked from a field the genesis amplifies at
streaming time, locked at their borders, skirted at the level changes, and swapped under D-017's
ꟻLIP thresholds; the horizon, the precision and the atmosphere from space are one Cesium test,
D-004 and D-023 as they stand.

> **State of the art in five sentences.** Every planet renderer since the virtual globes
> partitions the sphere into a quadtree of tiles, and the cube with an equal-angle warp (S2,
> Zucker & Higashi) won over the geographic grid (Cesium's two root tiles) and over HEALPix
> because its cells are square, nearly equal, and nest by arithmetic on their ids. On each tile
> the LOD is one of three heightfield-specific schemes — nested regular grids re-centred on the
> viewer (Losasso & Hoppe, on the sphere Clasen & Hege), a quadtree of grid patches whose
> vertices morph toward the parent (Strugar's CDLOD), or a longest-edge bisection kept on the GPU
> in a concurrent binary tree (Dupuy; "planetary scale geometry out of very coarse meshes" in
> under 0.2 ms) — or the general answer, a cluster hierarchy with a continuous cut (Nanite, and
> Epic's landscape since 5.3), which is what Forge has. The horizon is culled by testing one point
> per tile against the planet's sphere in a scaled space (Cesium's ellipsoidal occluder), cracks
> between levels are hidden by skirts (Ulrich, quantized-mesh) or removed by a morph, and depth is
> a reversed float buffer, which Outerra and Reed showed matches a logarithmic one for free. The
> shipped planet games (Elite, No Man's Sky, Star Citizen, Space Engineers, KSP2) all generate
> tiles at run time from noise around a cube-sphere or voxel quadtree, store nothing global and
> pay for it in geology, while Proland and Flight Simulator stream a coarse global dataset and
> refine it on demand, which is Forge's plan with a generated dataset. The literature measures
> triangles per pixel, tiles resident by level, the pool's turnover and the frame time at three
> altitudes; the owner will measure the pop first, so the ꟻLIP between the frames before and
> after a tile swap is the acceptance test.

**Contents**

1. [Partitions of the sphere: the cube, its warps, and the alternatives](#1-partitions-of-the-sphere-the-cube-its-warps-and-the-alternatives)
2. [LOD schemes on the sphere](#2-lod-schemes-on-the-sphere)
3. [Precision, the horizon, and the seams](#3-precision-the-horizon-and-the-seams)
4. [Streaming and generation on demand](#4-streaming-and-generation-on-demand)
5. [Atmosphere and lighting from orbit](#5-atmosphere-and-lighting-from-orbit)
6. [Measuring](#6-measuring)
7. [What professional engines do](#what-professional-engines-do)
8. [Recommendation for Forge](#recommendation-for-forge)
9. [What the numbers say](#what-the-numbers-say)
10. [Checked and left out](#checked-and-left-out)
11. [Verification notes](#verification-notes)

---

## 1. Partitions of the sphere: the cube, its warps, and the alternatives

D-037 fixed the partition: six cube faces, each a quadtree of cells of equal angle, ids of
kind–level–face–x–y in 64 bits. This section records why that is the consensus and what the two
alternatives (the geographic quadtree and HEALPix) would have cost.

**Matt Zucker, Yosuke Higashi. "Cube-to-sphere Projections for Procedural Texturing and Beyond."
*Journal of Computer Graphics Techniques* 7(2), 2018, 1–22.** [paper] [still-current]
<https://jcgt.org/published/0007/02/01/>

The comparison of cube-to-sphere warps on area distortion and GPU cost, re-verified for
`large-worlds.md` §8 and `terrain-genesis.md` §5: the tangent (equi-angular) warp is the one "to
maximize ease of implementation", the fifth-order polynomial the drop-in if `tan`/`atan` ever
shows in a profile. The gnomonic (unwarped) cube has corner cells about 5× the area of centre
cells; the tangent warp brings the spread to a few percent, which `partition.rs`'s test checks
at 2 %.
*Bearing:* nothing to decide; `CubeSphere::face_coords` is this warp with `dmath::atan` (D-016).
For tiles it means a tile's sample spacing varies by a few percent across a face and its D8 edge
lengths come from the projection, so the tile record carries its own spacing in metres, never a
constant per level.

**Google. "S2 Cell Hierarchy." S2 Geometry developer guide; the library on GitHub (Apache-2.0).**
[docs] [code] [still-current]
<https://s2geometry.io/devguide/s2cell_hierarchy> · <https://github.com/google/s2geometry>

Six faces, a quadratic warp, thirty levels to about a centimetre, ids of face bits plus two bits
per level on a Hilbert curve (the guide confirmed by search for `large-worlds.md` §8; the
repository's README only points to it). D-037 keeps the faces and the levels and drops the
Hilbert order ("locality on disk is the page pool's business, not the id's"); `CellId`'s parent
and children are shifts on the `x`, `y` bits.
*Bearing:* the tile's key on disk and in the cache is `CellId`'s `u64`; the neighbourhood read
the Hilbert order would have bought is what the clipmap's prefetch (§4) does explicitly.

**Cesium. quantized-mesh (the terrain tile format, on GitHub); Patrick Cozzi, Kevin Ring, *3D
Engine Design for Virtual Globes*, A K Peters/CRC Press, 2011.** [code] [book] [foundational]
[still-current]
<https://github.com/CesiumGS/quantized-mesh> · <https://www.virtualglobebook.com/>

The geographic quadtree, read from its format: "two root tiles at zoom level 0 (covering
−180°–0° and 0°–180° longitude)", tiles of 16-bit quantised `u`, `v`, `height` delta- and
zig-zag-encoded, an 88-byte header with the tile's centre, height range, bounding sphere and a
**horizon occlusion point** ("If this point is below the horizon, the entire tile is below the
horizon"), and four edge-index lists because "it is helpful to know which vertices are on the
edges in order to add skirts to hide cracks between adjacent levels of detail". Its cost is the
poles: a geographic cell's width shrinks with the cosine of the latitude, so the pole rows are
slivers and their tiles many for little ground.
*Bearing:* the tile header to copy, not the tiling: centre, height range, bounding sphere,
occlusion point, edge lists. The pole problem is why D-014 chose the cube and why a geographic
planet variant is not worth a flag.

**Krzysztof M. Górski, Eric Hivon, Anthony J. Banday, Benjamin D. Wandelt, Frode K. Hansen,
Martin Reinecke, Matthias Bartelmann. "HEALPix: A Framework for High-Resolution Discretization
and Fast Analysis of Data Distributed on the Sphere." *The Astrophysical Journal* 622(2), 2005,
759–771; with Rolf Westerteiger, Andreas Gerndt, Bernd Hamann, "Spherical Terrain Rendering
using the hierarchical HEALPix grid", *Proceedings of IRTG 1131 Workshop 2011* (OASIcs), 13–23.**
[paper] [still-current]
<https://iopscience.iop.org/article/10.1086/427976> (DOI 10.1086/427976) ·
<https://drops.dagstuhl.de/entities/document/10.4230/OASIcs.VLUDS.2011.13> (DOI
10.4230/OASIcs.VLUDS.2011.13)

The alternative with exact equal areas: "HEALPix—the Hierarchical Equal Area isoLatitude
Pixelization—is a versatile structure for the pixelization of data on the sphere", twelve base
diamonds subdivided in quadtrees, every pixel of the same area, the centres on rings of constant
latitude. Westerteiger, Gerndt and Hamann rendered Mars on it: "a hierarchical subdivision of the
HEALPix coordinate system using quadtrees" that "avoids singularities and allows for efficient
fusion of mixed-resolution digital elevation models and imagery", the projection done "within a
GPU shader".
*Bearing:* not adopted, and here so nobody re-derives the omission: the diamonds are rhombi, so a
grid tile on one has sheared D8 neighbourhoods and leaning normal cones, for an area gain of a
few percent over the tangent warp. Where HEALPix would pay is a global integral (the climate
bake's insolation, `planet-environment.md`), where equal areas remove a weight.

**Robert Kooima, Jason Leigh, Andrew Johnson, Doug Roberts, Mark SubbaRao, Thomas A. DeFanti.
"Planetary-Scale Terrain Composition." *IEEE Transactions on Visualization and Computer Graphics*
15(5), 2009, 719–733.** [paper] [still-current]
<https://dl.acm.org/doi/10.1109/TVCG.2009.43> (DOI 10.1109/TVCG.2009.43)

The virtual-globe answer to unregistered data: "a GPGPU process to tessellate spherical height
fields" and "a render-to-vertex-buffer technique to operate upon polygonal surface meshes in
image space", so height maps and imagery of different projections and resolutions blend
"regardless of boundary" with "smooth interpolation of levels of detail in both geometry and
imagery", out of core, "at scales approaching one meter".
*Bearing:* the composition problem is Forge's island-on-a-planet problem (§8, step 6): a fine
field placed over a coarse one, blended in a band; Forge does it once per tile and caches it.

---

## 2. LOD schemes on the sphere

Three heightfield-specific families and the general one, ordered by how much of the problem they
solve on the GPU; where Forge's cluster DAG per tile sits among them is the last entry's question.

**Peter Lindstrom, Valerio Pascucci. "Visualization of Large Terrains Made Easy." *IEEE
Visualization 2001*, 363–370; and "Terrain Simplification Simplified", *IEEE TVCG* 8(3), 2002.**
[paper] [foundational]
<https://dl.acm.org/doi/10.5555/601671.601729> · <http://www.pascucci.org/pdf-papers/vis2001.pdf>
(SOAR, Lawrence Livermore National Laboratory)

The restricted-quadtree line that CDLOD and the concurrent binary trees descend from: a
longest-edge-bisection hierarchy of the grid refined by nested error spheres (a vertex's sphere
contains its children's, so a top-down traversal never leaves a crack), with an out-of-core
layout that interleaves the levels so a page holds a subtree. The abstract could not be read
today (§11); the description is the standard one.
*Bearing:* the nesting rule is the one the cluster DAG already obeys ("a group's error is at
least its children's, its sphere contains theirs", `lod.rs`), which is why a DAG cut has no
cracks within a tile; the bisection is what the CBT entry below does on the GPU.

**Thatcher Ulrich. "Rendering Massive Terrains using Chunked Level of Detail Control." SIGGRAPH
2002 course *Super-size it! Scaling up to Massive Virtual Worlds*; code and notes on tulrich.com
(public domain).** [talk] [code] [foundational]
<https://tulrich.com/geekstuff/chunklod.html>

"A quadtree of 'chunks'", "each chunk is a rectangular, precomputed section of optimized
geometry", and "skirts to fill cracks rather than force each chunk edge to match"; the chunk's
vertices also morph toward their parent's positions as the split approaches, so a chunk swap
moves nothing (the origin of CDLOD's morph).
*Bearing:* the closest ancestor of "a tile is a precomputed mesh", with the chunk's mesh replaced
by a DAG cut. Two of its rules carry over in §8: the skirt for the level change, and the swap
only when the finer chunk's arrival changes nothing a pixel can see.

**Frank Losasso, Hugues Hoppe. "Geometry Clipmaps: Terrain Rendering Using Nested Regular Grids."
*ACM Transactions on Graphics* 23(3) (SIGGRAPH 2004), 769–776; Malte Clasen, Hans-Christian Hege,
"Terrain Rendering using Spherical Clipmaps", *EuroVis 2006* (Lisbon), 91–98; Aleksandar M.
Dimitrijević, Dejan D. Rančić, "Ellipsoidal Clipmaps – A planet-sized terrain rendering
algorithm", *Computers & Graphics* 52, 2015, 43–61 (and "High-performance Ellipsoidal Clipmaps",
2023).** [paper] [foundational] [still-current]
<https://hhoppe.com/proj/geomclipmap/> ·
<https://diglib.eg.org/items/7b198898-fef4-4d95-9e5b-2141938ad59b> (DOI
10.2312/VisSym/EuroVis06/091-098) ·
<https://www.sciencedirect.com/science/article/abs/pii/S0097849315000916> (DOI
10.1016/j.cag.2015.06.006)

Nested grids around the viewer (re-verified in `terrain-genesis.md` §5 with its "40GB height
map of the United States" at 100:1). Clasen and Hege put them on the sphere: a static set of
triangles in a viewer-centred spherical parameterisation, the height sampled by "mapping of
texture coordinates to calculate the height map sample position based on the static vertex offset
and the variable view position", so the rings never re-mesh and the poles are wherever the viewer
is not. Dimitrijević and Rančić extend it to the ellipsoid, "divided into three partitions that
are seamlessly stitched together", the grid "generated on the fly in the vertex shader".
*Bearing:* the cheapest planet far field (one static mesh, a height texture per level) and the
right mesh for the *ocean* sphere (`water.md`), but a clipmap draws the same density in every
direction and cannot skip the back of a mountain, where the DAG cut draws by projected error and
culls by cone and occlusion; kept as the fallback if the DAG per tile fails from orbit.

**Filip Strugar. "Continuous Distance-Dependent Level of Detail for Rendering Heightmaps
(CDLOD)." 2010; source, paper and data on GitHub (MIT).** [paper] [code] [still-current]
<https://github.com/fstrugar/CDLOD>

The quadtree of grid patches "selected by precise three-dimensional distance between the
observer and the terrain", vertices morphed toward the parent so there is no stitching and no
pop (read on GitHub for `terrain-genesis.md` §5). D-014 named it the planet's far field.
*Bearing:* the morph is the idea to keep, not the mesh: the pop CDLOD removes is the one between
*its own* levels, which the DAG cut removes by construction inside a tile; between two *tiles* of
different levels Forge has no morph (a cut is a set of clusters, not a vertex function), so §8
replaces it with a swap criterion in ꟻLIP. D-014's "CDLOD far field" is amended once §9 holds.

**Jad Khoury, Jonathan Dupuy, Christophe Riccio. "Adaptive GPU Tessellation with Compute
Shaders." *GPU Zen 2* (Wolfgang Engel, ed.), 2019; Jonathan Dupuy, "Concurrent Binary Trees
(with application to longest edge bisection)", *PACMCGIT* 3(2) (HPG 2020), Article 21; Anis
Benyoub, Jonathan Dupuy, "Concurrent Binary Trees for Large-Scale Game Components", *PACMCGIT*
7(3) (HPG 2024); the demos on GitHub (public domain / MIT).** [paper] [code] [recent]
<https://www.semanticscholar.org/paper/2834da6916bf253bd0686187ae53175959a2264f> ·
<https://dl.acm.org/doi/10.1145/3406186> · <https://dl.acm.org/doi/10.1145/3675371> ·
<https://github.com/jdupuy/LongestEdgeBisection2D> · <https://github.com/jdupuy/libleb>

The GPU-driven line. The 2019 chapter refines "coarse meshes as they get closer to the camera"
with "a GPU-based refinement scheme that allows arbitrary subdivision levels at constant memory
costs", each polygon's implicit subdivision kept in "a compact, double-buffered array". The 2020
paper replaces the array with the concurrent binary tree, "a binary heap (a 1D array) that
explicitly stores the sum-reduction tree of a bitfield", so a million-leaf bisection splits and
merges in parallel with no CPU; the repository's second program is "a terrain renderer based on
the adaptive longest edge bisection", CPU or GPU. The 2024 paper takes it to half-edge meshes
and reports "planetary scale geometry out of very coarse meshes" in under 0.2 ms on consoles
(`large-worlds.md` §8).
*Bearing:* the strongest competitor to a DAG per tile, and different in kind: a CBT refines a
*function* every frame, so it needs the height as a texture the GPU can sample anywhere (a 273²
`f32` field is 300 KB) and never cooks geometry, at the clipmap's price of no per-cluster
occlusion or cone culling plus a subdivision pass per frame. §8 keeps it as the spike to run
against the DAG once both exist, with the same field and the F1 overlay.

**Brian Karis, Rune Stubbe, Graham Wihlidal. "A Deep Dive into Nanite Virtualized Geometry."
SIGGRAPH 2021 *Advances in Real-Time Rendering in Games*; Epic Games, "Using Nanite with
Landscapes" (5.3+), "Nanite Tessellation" (5.4, experimental; `r.Nanite.AllowTessellation`,
`r.Nanite.Tessellation`) and the Virtual Heightfield Mesh plugin (4.26+), Unreal Engine
documentation.** [talk] [docs] [recent]
<https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf> ·
<https://dev.epicgames.com/documentation/unreal-engine/using-nanite-with-landscapes-in-unreal-engine>
· <https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/VirtualHeightfieldMesh>

The general answer and its three terrain forms. Nanite Landscape (5.3) cooks each landscape
component into the cluster hierarchy, "rebuilt in the background" when sculpted; Nanite
tessellation (5.4) is "the ability to displace the surface of a mesh or landscape as it is
rendered to create the appearance of more detail than the mesh contains", so the clusters carry
the large shape and a displacement map the small one; the older Virtual Heightfield Mesh draws a
heightfield from a runtime virtual texture ("the World Position Z value plus the material's
displacement") as a GPU-tessellated grid with no cooked geometry.
*Bearing:* the three forms are the three stages of Forge's own answer: cooked clusters today (the
island at 4 m is one cluster-DAG mesh), the same per tile next (§8), and later, if the finest
level's pages cost too much, a displacement stage for the last octave (the 2 m amplification as
a texture over the 4 m tile), which is Nanite tessellation's split. The Virtual Heightfield Mesh
form is the CBT entry's form and is measured with it.

**Forge's cluster DAG as a terrain LOD (issues #35, #36, #96; D-025; `forge_geom::lod`,
`forge_geom::page`, `forge_geom::city::PropKind::Heightfield`).** [code]

What exists: a heightfield becomes a grid mesh (`heightfield_mesh`), cooked into clusters of at
most 128 triangles, groups of 8 simplified to half per level with group borders locked and the
mesh's outer edge "locked like a group border" (173 roots along the city ground's edge), errors
and spheres monotonic up the DAG so the cut has no cracks, 128 KiB pages holding whole groups so
any resident set containing the roots is watertight, residency by need. The city's ground is
2001² at 2 m: 8 M triangles, 186 k clusters, 14 levels, 12 s to cook, 1.05 ms a frame with a
million props at 1600 × 900; the island at 4 m is four times it (33.5 M triangles).
*Bearing:* a tile is this with three additions, none to the cook: the field comes with a halo so
border normals agree with the neighbour's, a skirt is appended for the level changes, and the
tile's instance carries a horizon occlusion point. What the DAG lacks against the heightfield
schemes, a vertex morph between tiles, is what §8's swap criterion replaces.

---

## 3. Precision, the horizon, and the seams

**Patrick Cozzi, Kevin Ring, *3D Engine Design for Virtual Globes* (2011); Nathan Reed, "Depth
Precision Visualized" (2015); Brano Kemen, "Maximizing Depth Buffer Range and Precision", Outerra
blog (2012); Forge's D-004 and its 🟡 amendment (#93).** [book] [web] [still-current]
<https://www.virtualglobebook.com/> · <https://www.reedbeta.com/blog/depth-precision-visualized/> ·
<https://outerra.blogspot.com/2012/11/maximizing-depth-buffer-range-and.html>

Settled in `large-worlds.md` §1; repeated only for what the planet adds. Positions: a tile's
vertices are `f32` local to the tile's centre (a 1.15 km tile resolves to 0.1 mm), the tile's
origin is an `f64` point of the planet's frame, and the instance table's `(int3 cell, float3
local)` form of the #93 amendment makes the camera-relative subtraction exact; nothing is
rebased. Depth: `D32_SFLOAT` reversed with an infinite far plane, which Reed showed gives a zero
comparison-error rate and Outerra found "nearly matches" a 24-bit logarithmic buffer for free.
The planet adds the *near* side: from orbit the near plane can sit at 1 km, on the ground at
0.1 m, so it follows the camera's height above the tile under it, a per-frame constant.
*Bearing:* no new decision. The one measurement to add to #93's `tools/origins.sh` is a planet
case: the island's captures at 10⁴ to 10⁷ m from the origin become captures at the same spot on
a 1 500 km planet 10¹¹ m from its star.

**Kevin Ring (Cesium). "Horizon Culling", 25 April 2013, and "Computing the horizon occlusion
point", 9 May 2013; `EllipsoidalOccluder.js` in CesiumJS (Apache-2.0).** [web] [code]
[still-current]
<https://cesium.com/blog/2013/04/25/horizon-culling/> ·
<https://cesium.com/blog/2013/05/09/computing-the-horizon-occlusion-point/> ·
<https://github.com/CesiumGS/cesium/blob/main/packages/engine/Source/Core/EllipsoidalOccluder.js>

"Horizon culling is the straightforward idea that you do not need to render objects that lie
below the horizon as viewed from the current viewer position", made fast by working in the space
where the ellipsoid is a unit sphere: with the camera at `c` (scaled) and `v = c / |c|²` the
point where the horizon cone's axis meets the horizon plane, a point is hidden when two dot
products say it lies beyond that plane and inside the cone, no square root (the code's comments:
"If vhMagnitudeSquared < 0 then we are below the surface of the ellipsoid"). The second post
computes, for a tile, the one point whose visibility implies the tile's: "If the point is below
the horizon, all of the positions are guaranteed to be below the horizon as well"; quantized-mesh
stores it per tile (§1).
*Bearing:* the test goes in two places. In the instance cull, for instances bound to a planet:
the occlusion point against the planet's sphere before the frustum test, so the far hemisphere's
tiles cost one compare each instead of a cluster cull (the HZB would remove them too, a frame
late and after their clusters were counted). In the streaming plan: a wanted cell whose highest
point (the field's maximum, in the record) is below the horizon keeps its slot but drops to the
lowest priority, so at low altitude the coarse levels' reach does not pull tiles nobody can see.

**Cesium. 3D Tiles specification: geometric error and screen-space error (OGC community
standard; GitHub).** [docs] [code] [still-current]
<https://github.com/CesiumGS/3d-tiles/blob/main/specification/README.adoc>

The vocabulary of the level choice, read from the specification: "A tile's geometric error
defines the selection metric for that tile. Its value is a nonnegative number that specifies the
error, in meters, of the tile's simplified representation of its source geometry"; an
implementation "will consider a maximum allowed Screen-Space Error (SSE), the error measured in
pixels"; and "if the tile has replacement refinement, the children tiles are rendered in place
of the parent", against additive refinement.
*Bearing:* the DAG's cut is SSE with replacement refinement at cluster granularity, and D-037's
clipmap is replacement refinement at tile granularity with the rule that the finest resident cell
draws. Forge's threshold is the cull's 1 px; the tile record's geometric error is the maximum
height difference to its parent's field, which is also the skirt's depth.

**Skirts, locked borders and morphs (a synthesis: Ulrich 2002, quantized-mesh, CDLOD, Forge's
`lod.rs`; no new source).**

Three ways to meet a neighbour of another level. A *skirt* is a fence of triangles hanging from
the tile's edge down by at least the level's geometric error (Ulrich; quantized-mesh's edge lists
exist "to add skirts to hide cracks between adjacent levels of detail"; the Godot demo in §4 says
the same); it costs `4 × (n − 1)` extra quads a tile, hides the crack under the coarser
neighbour's surface, and shows a sliver of wrong shading only at a grazing angle. A *locked
border* (Forge's cook) keeps the border's vertices through every level, so two tiles of the same
level match to the bit at any cut; it cannot help across levels, where the fine tile has samples
the coarse one lacks. A *morph* (CDLOD) moves the fine tile's odd border vertices onto the coarse
edge; it needs a vertex function, which a DAG cut is not. The combination that costs least and
leaves no crack: locked borders for equal levels, a skirt for unequal ones, no morph.
*Bearing:* §8's tile mesh is the 257² grid plus its skirt, locked at the grid's edge, the skirt's
bottom free to simplify; a `--show-skirts` tint on the layered material makes any visible fence
obvious on a capture.

**Normal maps versus geometry at distance (Frontier's first-release planets; Outerra; Forge's
resolve).** [web] [still-current]
<https://80.lv/articles/generating-the-universe-in-elite-dangerous> ·
<https://outerra.blogspot.com/2009/02/procedural-terrain-algorithm.html>

Elite's first landable planets were "perfect spheres with height difference provided by normal
mapping" (Doc Ross, 80.lv), and Outerra refines coarse data "by fractal-based procedural
techniques down to centimeter-level details", geometry near and shading far. In Forge the DAG cut
stops refining at 1 px of error, so a far tile is drawn from its coarse clusters while its *layer
map* (D-028) and the material's normal texture carry what is finer than a pixel.
*Bearing:* no separate normal-map path for far tiles; the layer map at a texel per 4 m and the
material's detail normals are the "normal map at distance", and the F1 overlay's triangles per
pixel (§6) says whether the cut stops where it should. The one addition worth its cost is a
per-tile *macro normal* at the coarse levels (the field's Horn gradient at the tile's spacing,
packed with the layer map), so a tile drawn from two clusters still shades its ridges from orbit.

**Epic Games. "World Partition — Hierarchical Level of Detail." Unreal Engine documentation.**
[docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/world-partition---hierarchical-level-of-detail-in-unreal-engine>

"Content in unloaded cells is replaced with HLODs" (confirmed for `large-worlds.md` §5): a
merged, simplified or instanced proxy per cell, generated at cook time, so the far view survives
the streaming.
*Bearing:* D-037's clipmap makes the coarse tiles the HLOD: a level-*ℓ* tile *is* the proxy of
its four children, cooked from the same field at half the resolution. What still needs an HLOD is
what stands *on* the tiles (trees, rocks, the city), which `vegetation-materials.md`'s impostors
and `large-worlds.md` §7's aggregates cover; the tile record reserves a slot for a proxy of its
props so the two arrive together.

---

## 4. Streaming and generation on demand

The shipped planets, the open source, and what Forge's job system and page pool already fix.

**Éric Bruneton, Fabrice Neyret. "Real-Time Rendering and Editing of Vector-based Terrains."
*Computer Graphics Forum* 27(2) (Eurographics 2008); Proland 4.0 (INRIA, BSD-3), its core and
terrain documentation.** [paper] [code] [docs] [still-current]
<https://maverick.inria.fr/Publications/2008/BN08/> · <https://proland.inrialpes.fr/> ·
<https://proland.inrialpes.fr/doc/proland-4.0/core/html/index.html>

The reference architecture for a generated planet, and the one Forge's plan is closest to.
Proland's core is "a producer framework, a terrain framework, and a basic user interface
framework": a terrain is "a terrain quadtree that is dynamically subdivided based on the current
viewer position", every quad's data is made by a *producer* per layer on demand, the whole
"based on a task graph and a cache manager that takes advantage of multi-processors, supports
data prefetching to reduce disk latencies, and automatically manages dependencies between
tasks", with "all data loaded or generated on the fly according to the viewpoint"; the 2008
paper's rivers and roads are vectors rasterised into the quads that need them.
*Bearing:* the vocabulary of §8's data model: a tile is produced by a chain of producers (coarse
field → amplified field → layer map → mesh → pages), each cached by content hash, each a job with
dependencies on `forge-task`'s counters; the island's rivers and lakes (`hydrology`) are
rasterised into the tiles their footprint crosses. BSD, so it may be read line by line; its
producers run on the GPU where Forge's run on the CPU for D-016.

**Brano Kemen et al. Outerra: "Procedural terrain algorithm" (February 2009) and the elevation
data posts (2012–2015).** [web] [still-current]
<https://outerra.blogspot.com/2009/02/procedural-terrain-algorithm.html> ·
<https://outerra.blogspot.com/2015/05/evaluation-of-30m-elevation-data-in.html>

Real-scale Earth from "elevation data with resolution 90m where available, 1km resolution for
oceans", "further refined by fractal-based procedural techniques down to centimeter-level
details" at run time, per tile, on the GPU; the 2009 post's refinement reads the coarse level's
slope so ridges sharpen and valleys stay smooth (confirmed by search for `terrain-genesis.md`).
*Bearing:* the two-scale pattern, coarse data plus deterministic refinement per tile, is Forge's,
with two differences that follow from D-016 and the genesis: the refinement is erosion that
respects the coarse drainage (Schott 2024's amplification) rather than noise, and it runs on the
CPU so the server's collision heights are the client's bytes.

**Doc Ross (Frontier), "Generating the Universe in Elite: Dangerous", 80.lv, 2018; Frontier's
Horizons planet video with John Patoney and Matt Dickenson (2015, wccftech's write-up); PC Gamer,
"Here's how Frontier rebuilt a galaxy's worth of planets for Elite Dangerous: Odyssey" (2021).**
[web] [still-current]
<https://80.lv/articles/generating-the-universe-in-elite-dangerous> ·
<https://wccftech.com/planets-elite-dangerous-horizons/> ·
<https://www.pcgamer.com/heres-how-frontier-rebuilt-a-galaxys-worth-of-planets-for-elite-dangerous-odyssey/>

Planets "start as a cube with square sub-dividing faces that behave as quadtrees";
"planet-specific average properties are packed into a buffer sent to the GPU, where they modulate
noise functions combined to form geological shapes"; the generator "must be predictable and
deterministic" because nothing is stored. Horizons (2015) landed on airless worlds; Odyssey (2021)
regenerated every landable planet with a new tech (the PC Gamer piece; its wording was not read,
§11).
*Bearing:* the run-time half of the plan, on the GPU and with no global pass; the Odyssey rebuild
is the evidence that noise alone stops looking right once the player walks, which is what the
coarse genesis is for. Elite's determinism rule is D-016 at galactic scale.

**Innes McKendrick (Hello Games). "Continuous World Generation in 'No Man's Sky'." GDC 2017;
Sean Murray, "Building Worlds Using Math(s)", GDC 2017.** [talk] [still-current]
<https://www.gdcvault.com/play/1024265/Continuous_World_Generation_in__No_Man_s_Sky_> ·
<https://www.gdcvault.com/play/1024514/Building-Worlds-Using>

McKendrick: "voxel-based world generation, through polygonization and texturing, to eventual
population and simulation", "continuously in real-time" (confirmed for `terrain-genesis.md` §4).
Murray: "generating both realistic and alien terrains without artistic input, using
mathematics", and the point that an infinite world is tested statistically, by automated probes
and the distributions of what they find, not by visiting.
*Bearing:* voxels because the game digs; Forge's planet is a heightfield with SDF bricks where it
is edited (D-014), so tiles stay 2D. And the test method: a `planet --survey` mode that generates
a fixed sample of tiles per face and level and prints their statistics (slope histogram, drainage
reaching the sea, lake fraction, digest) is how a planet nobody can walk is checked, in CI.

**Cloud Imperium Games. "Planet Tech v4" (CitizenCon 2019, "Terra Firmer"; Alpha 3.8) and
"Planet Tech v5" / Genesis (CitizenCon 2954, 2024; Alpha 4.x), as the Star Citizen Wiki records
them.** [web] [recent]
<https://starcitizen.tools/Planet_Tech_v4> ·
<https://starcitizen.tools/CitizenCon_2019_-_Terra_Firmer> · <https://starcitizen.tools/Planet_Tech_v5>

v4: with the climate layers (`terrain-genesis.md` §4), "LOD transitioning became significantly
more fluid with less pop-ins and no global textures, allowing visibility of actual ground texture
from outer space", and "terrain simulation and displacement of sand and soil based on wind". v5
(Genesis): "physically-based rules with Genesis data pools to emulate nature", the planetary
data extended to "Geology, Soil Type, Soil Depth and Nutrients", driving "the assignment and
unrepetitively tiled blending of terrain textures", flora "based on competition rules", and "the
placement of rocks and debris derived from erosion simulation". The talks are videos; the wiki
is a fan record (§11).
*Bearing:* the only shipped planet tech still being developed is moving toward Forge's plan: a
global simulation whose *fields* drive materials and placement at run time, and the ground's own
material seen from orbit instead of a global colour texture; the coarse genesis emits those fields.

**Eric DeFelice (Intercept Games). "Developer Insights #12 – Planet Tech." Kerbal Space Program
forums, December 2021; with "Developer Insights #18 – Graphics of Early Access KSP2" (2023).**
[web] [recent]
<https://forum.kerbalspaceprogram.com/topic/205930-developer-insights-12-%E2%80%93-planet-tech>
· <https://forum.kerbalspaceprogram.com/topic/214806-developer-insights-18-graphics-of-early-access-ksp2>

A graphics engineer's account of "how the team generates, positions, and renders the planets" in
KSP2, replacing the first game's procedural quad sphere; community discussion of the early-access
build describes the new terrain as a concurrent-binary-tree system, which could not be confirmed
from the post itself (the forum is unreachable, §11); the studio closed in 2024.
*Bearing:* the one shipped-adjacent data point for a CBT terrain on a planet, if the description
holds; worth reading on a full network before the CBT spike of §8 (the bisection against D-009).

**Lionel Fuentes (Asobo Studio). "Advanced Graphics Summit: Designing the Terrain System of
'Flight Simulator': Representing the Earth." GDC 2022 (GDC Vault 1027581; slides with notes on
asobostudio.com); with the Blackshark.ai coverage (TechCrunch 2020).** [talk] [recent]
<https://www.gdcvault.com/play/1027581/Advanced-Graphics-Summit-Designing-the> ·
<https://www.asobostudio.com/files/inline-images/Designing_Terrain_System_Fuentes_Lionel.pdf> ·
<https://techcrunch.com/2020/08/17/meet-the-startup-that-helped-microsoft-build-the-world-of-flight-simulator>

The technical director's account of "the hybrid procedural/real-world data methods" behind a
planet that streams "more than 2 petabytes of data in real-time to the game client": Bing's
photogrammetry as "1:1 replicas of 400 cities", elsewhere "1.5 billion buildings from 2D
satellite images", the terrain a quadtree of tiles whose height, imagery and vector data are
fetched by level from the cloud and refined procedurally below the data's resolution. The deck's
own numbers were not read today (§11).
*Bearing:* the streaming shape at the extreme: every level of every layer is a separately fetched
tile, the cache is the whole game, and the procedural detail below the data's resolution is what
makes 30 m data look like 30 cm from a cockpit. Forge's tiles replace the fetch with a producer
and keep the rest.

**Marek Rosa (Keen Software House). "Planets! Because you wanted them", 12 November 2015, with
Ondřej Petržilka's April 2015 guest post; Update 01.108.** [web] [still-current]
<https://blog.marekrosa.org/2015/11/planets-because-you-wanted-them_12/> ·
<https://blog.marekrosa.org/2015/04/guest-post-by-ondrej-petrzilka-space_17/>

Voxel planets grown from the asteroid generator: "three planets (up to 120 km in diameter
inspired by Earth, Mars and an alien world), three moons (up to 19 km in diameter)", "large,
immobile, destructible voxel objects".
*Bearing:* the small-planet end of the design space: at 120 km the horizon is 1.2 km away from
60 m up and the curvature is in every screenshot, a look rather than a compromise for a game that
digs. Forge's 1 500 km test planet (D-037) is another regime; the demo exposes the radius so the
owner sees both.

**Open source planet demos: cuberact's "Procedural Planet – Chunked LOD" (Godot 4.6, GDScript,
MIT); Hoimar's Planet-Generator (Godot 3, MIT); Zylann's Solar System demo (Godot 4, Voxel
Tools); Sebastian Lague's Procedural-Planets (Unity, MIT); Sven Forstmann's Planet-LOD (MIT);
TokisanGames' Terrain3D (Godot 4, MIT); and Manuel Zechmann, Helmut Hlavacs, "Comparative
Analysis of Procedural Planet Generators", GAME-ON 2025 (arXiv:2510.24764).** [code] [paper]
[recent]
<https://github.com/cuberact/godot-cuberact-planet-chunked-lod> ·
<https://github.com/Hoimar/Planet-Generator> · <https://github.com/Zylann/solar_system_demo> ·
<https://github.com/SebLague/Procedural-Planets> · <https://github.com/sp4cerat/Planet-LOD> ·
<https://github.com/TokisanGames/Terrain3D> · <https://arxiv.org/abs/2510.24764>

Read on GitHub. The cuberact demo is the textbook in one file: a spherified cube, 17 × 17 chunks
to 20 levels, a split when "the chunk appears 'too large' on screen" (an angular threshold with
hysteresis, "8 per frame"), noise displaced in the vertex shader, "the fix is a **skirt**: an
extra ring of triangles hanging below each chunk edge, pushed toward the planet center", and
precision by moving the world: "Every frame, the planet's position is adjusted so the active
camera stays at (or very near) the world origin." Hoimar's addon is the same quadtree with
"seamless terrain patches"; Zylann's demo has voxel planets "1 to 2 Km in radius" with origin
shifting; Lague's is the Unity cube sphere from layered noise; Planet-LOD an icosahedron
subdivided by distance; Terrain3D Godot's serious *flat* terrain ("up to 10 levels of detail",
regions up to "65.5x65.5km"). Zechmann and Hlavacs built two Godot generators with "a
quadtree-based Level of Detail (LOD) system" and ran a user study against two existing projects.
*Bearing:* none is an engine, and together they fix the floor: cube sphere, quadtree, skirts, GPU
noise, and an origin trick Forge has ruled out (D-004). What they lack is exactly the list Forge
adds: a global pass, a cluster hierarchy, a page pool, a digest per tile. Useful as test scenes to
compare a look against, and cuberact's file as the spec of what a first planet must do.

**Forge's streaming rules as they apply to tiles (ARCHITECTURE §3, D-018, D-025, issue #77,
`memory-streaming.md` §4 and its recommendation; no new source).**

The job system runs "procedural generation at `Low` priority in ≤ 200 µs jobs" beside the frame.
The page pool holds 128 KiB pages, loads the neediest first with `reads_in_flight` reads out and
`upload_pages` copies a frame, evicts the lowest need, keeps residency closed upwards, and copies
through `streaming/upload`, already on the transfer queue in the city. The memory research fixed
the prefetch sources ("cells along the velocity vector, cutscene cameras, teleports"), the ceiling
(64 MB a frame at 60 Hz on the transfer queue; the city's flight uses 200 KiB) and the failure
mode to design for (degrade to blur, never to hitches).
*Bearing:* the tile pipeline adds a producer in front of the pool, not a second streamer: a tile
is *generated* (amplify, layers, cook) by `Low` jobs into the disk cache, then its pages are
*streamed* by the existing pool as any mesh's are. The only new policy is speed: the finest
wanted level is capped by `speed × latency < reach`, so at 300 m/s and 0.5 s of latency the
1.15 km tiles are fine, and at 3 km/s of re-entry the finest level is 2.3 km and the surface
arrives as the ship slows, as in every planet game.

---

## 5. Atmosphere and lighting from orbit

Most of this section is built; the entries say what remains and point at the files that own it.

**Sébastien Hillaire. "A Scalable and Production Ready Sky and Atmosphere Rendering Technique."
*Computer Graphics Forum* 39(4) (EGSR 2020); Forge's D-023 with its planet-view table (#26) and
the inside-the-atmosphere tables (#43).** [paper] [still-current]
<https://sebh.github.io/publications/> (verified for `lighting-gi.md` §6)

What exists: the transmittance and multiple-scattering tables as graph passes, a per-pixel march
from space (16 segments, the planet's own shadow, the ground lit through the air plus the
multiple-scattering term as skylight), the planet-view table that makes the view from outside a
lookup ("the sky pass facing a 50° planet goes 0.333 → 0.105 ms"), and from inside the sky-view
table, the aerial-perspective volume to 8 km and the compose pass. The ground under the march is
today a sphere.
*Bearing:* the planet demo changes one input: the sky pass's ground becomes whatever the tiles
rasterised, so the aerial perspective applies to the terrain by its depth and the from-space
march takes the terrain's depth as its floor where a tile covers the pixel. The terminator comes
free (the sun's angle at the pixel's ground point); the night side is the multiple-scattering
term at zero plus the stars (city lights and airglow: `lighting-gi.md` §6). The number to check
is the aerial volume's 8 km against a horizon 1 000 km away from orbit: beyond it the compose
pass falls back to the per-pixel transmittance and luminance the march already computes.

**Jakub Boksanský, Michael Wimmer, Jiří Bittner. "Ray Traced Shadows: Maintaining Real-Time
Frame Rates." *Ray Tracing Gems* (Apress, 2019), ch. 13, 159–182; Forge's D-029 (a BLAS per mesh
from its DAG, a TLAS over the instances).** [book] [still-current]
<https://link.springer.com/chapter/10.1007/978-1-4842-4427-2_13> (DOI
10.1007/978-1-4842-4427-2_13; the authors' PDF on boksajak.github.io)

Shadow rays "can become a bottleneck for high-resolution rendering, multiple lights, or area
lights", so the chapter varies the ray count per pixel by a visibility variance estimate and
filters temporally, the discipline Forge's TAA-averaged sun-disc rays (#54) follow. What the
planet adds is scale in the acceleration structure, not in the rays: a TLAS over the resident
tiles' BLASes, each a cut of the tile's DAG (D-029; the city's BLAS set built in 8 ms).
*Bearing:* the terrain's self-shadow from orbit is the terminator and the ridges' long shadows
near it, from the coarse tiles; from the ground it is the near tiles at full detail. So the BLAS
per tile is built from the cut at 4× the screen error (a tenth of the triangles) in a
compute-queue pass (`terrain/blas`, #77), rebuilt when the tile's level changes, and the TLAS is
refit each frame over the ~200 resident tiles, the city's cost.

**Clouds, the ocean sphere and the weather (pointers).** [internal]

Clouds over the planet are `planet-environment.md` §4 (the Nubis weather map generated from the
weather function; the cloud layer itself is `lighting-gi.md` §6's item), and from orbit they are
the planet's cloud fraction per climate cell (the 78 km atlas, D-034) rendered as the same layer
seen from above. The ocean sphere is `water.md` (D-038 🟡): the FFT cascades near, Bruneton,
Neyret and Holzschuch's geometry-to-BRDF transition so the sun's glitter is stable from the deck
to the horizon and, with Dupuy and Bruneton's whitecaps, "for scales ranging from centimeter to
planetary in real time"; the mesh under it is the spherical clipmap of §2 on the sea-level
sphere, drawn where the tiles' field is below sea level. Neither needs more from this file than
the tile's `sea_level` and its coast distance, which the genesis writes.

---

## 6. Measuring

**What the literature measures, and what Forge's tools already print.** [internal, with Karis
2021 and the ꟻLIP repository]
<https://github.com/NVlabs/flip>

Nanite's target is a triangle per pixel and its overlay prints clusters and triangles per pass;
Forge's F1 overlay prints instances, clusters and triangles per cull pass, pages resident and
uploaded, and every graph pass's GPU time; `tools/timings.sh` writes them per run. ꟻLIP
(Andersson, Nilsson, Akenine-Möller, Oskarsson, Åström, Fairchild 2020, *PACMCGIT* 3(2); the
repository is "a tool for visualizing and communicating the errors in rendered images", BSD-3,
LDR and HDR variants) is `imgdiff`'s perceptual check since D-017's amendment, with the proposed
pass at 67 pixels per degree: every pixel below 0.15, the mean below 0.02.
*Bearing:* six numbers per capture and three altitudes. (1) **Triangles per pixel** on the
terrain, by tile level, from the cull's counters over the visibility buffer's covered pixels: the
target is 0.5–2, and a level above 2 has a cut stopping too late or a locked border too fine.
(2) **Tiles resident by level** and their pages, against D-037's twenty per level. (3) **The
pool**: pages uploaded and evicted a frame, the need of the last page kept, the frames in which a
wanted page was absent. (4) **Generation**: tiles amplified, cooked and read from the cache per
second, with the p50/p99 of a tile's latency from *wanted* to *pages resident*. (5) **The frame
at 1440p** (p50/p99/max, never a mean: P7) from orbit, at 10 km and on the ground, terrain apart
from the sky. (6) **The pop**: for every tile swap of a scripted descent, the LDR-ꟻLIP of the
frame after against the frame before, with the swap's level and distance; a peak over 0.15 is
the pop the owner will see, and the run fails on it as the culling harness fails on a pixel.
Determinism is the seventh: every tile's field digest (`Field2::digest`, FNV-1a today; BLAKE3
with D-018's container) is in its record, a fixed list of cells per face and level is generated
in CI at one and six workers, and the digests must match the golden list and the owner's
machine. The mesh cache is *not* under the digest: the field is the contract the server shares
(P8); the cook is the client's, deterministic for one binary.

---

## What professional engines do

- **Unreal Engine 5.x**: no planet, but every piece: Large World Coordinates (`double` in the
  engine, camera-relative on the GPU), World Partition's grid with HLOD per cell, the landscape on
  Nanite since 5.3, Nanite tessellation since 5.4, the Virtual Heightfield Mesh before it.
  Planets are plugins or Cesium for Unreal, "a high-accuracy full-scale WGS84 globe" whose
  `Cesium3DTileset` actor "streams 3D Tiles data" (quantized-mesh terrain among them).
- **Unity and Godot 4**: a flat terrain each (Unity's `TerrainData`, Godot's third-party
  Terrain3D); planets are third party (the cube-sphere demos above, Zylann's voxels).
- **UNIGINE**: double-precision coordinates "64-bit per axis" and a geographic mode, a Landscape
  Terrain of layered maps for "up to 10000km x 10000km areas" that "cannot be used to create a
  whole planet".
- **Star Citizen**: 64-bit positions, a cube-sphere planet tech in its fifth version with
  simulated soils and erosion-derived placement (§4).
- **Frontier's Cobra (Elite)** and **Hello Games (No Man's Sky)**: cube-sphere quadtrees with GPU
  noise, or voxels around the player; nothing stored, everything deterministic, tested
  statistically; Elite's planets rebuilt for walking in 2021.
- **Asobo (Flight Simulator)**: a quadtree of per-level, per-layer tiles fetched from the cloud
  and refined procedurally; the planet is data. **Keen (Space Engineers)**: voxel planets of
  120 km.
- **Proland / Outerra**: coarse real data plus per-tile refinement, producers on demand with a
  cache; the two that look most like Forge's plan with the data swapped for a genesis.

The pattern: nobody ships a planet from a mesh; everybody ships a quadtree of tiles made on
demand, and the ones that look right at walking distance have, or are adding, a global simulation.

---

## Recommendation for Forge

**The shape.** A planet is a `CubeSphere` (D-037) whose surface is tiles: one per cell of the
clipmap, each a cluster-DAG prop cooked from a `Field2` the genesis produces at that cell's
spacing, drawn by the existing renderer as an instance, streamed by the existing page pool. The
six level-0 tiles are always resident (the planet's roots, as a mesh's root pages are); the
clipmap loads levels 1 to the finest around every viewer; the finest resident cell draws. No
CDLOD, no CBT, no morph in the first build: the DAG cut is continuous inside a tile, locked
borders make equal levels exact, a skirt hides the level change, and a swap criterion in ꟻLIP
defines "no pop". D-014's "CDLOD far field" is amended to this once §9's numbers hold on the GPU.

**Build order.**

1. **The coarse planet graph from the genesis.** Stages 1–4 of `terrain-genesis.md` on the six
   warped faces: a `Field2` per face with a neighbour rule across face edges (the receivers, the
   basin graph, the stack and the flood are graph algorithms and need only that rule), edge
   lengths and areas from the projection, sea level as the outlet everywhere, uplift from a seeded
   plate field. First at 1025² per face (6.3 M cells, 2.3 km on the 1 500 km planet: the island's
   per-step cost scaled says 0.12 s a step on four cores, 150 steps in 20 s), then 2049² (25 M
   cells, 1.15 km, about a minute) as the 1 km target. Output: the coarse fields per face, cached
   by the seed's hash; `genesis --planet R --seed N` writes six hillshades and the digests.
2. **The tile producer.** `forge_procgen::tile::produce(planet, cell) -> Tile`, a pure function
   of the seed and the `CellId`: sample the parent level's field (level 0 samples the coarse
   graph) over the cell with a halo of 8 samples, upsample ×2, run stage 5 (thermal, a short pipe
   pass with the rivers as fixed inflows, deposition) with the halo, then stage 6 (the layer byte
   and weight per texel), and write the record below. Cached on disk under the cell id, in D-018's
   container when it exists. Deterministic under D-016 (pinned tie-breaks, `dmath`, work split by
   rows and merged by index); the digest is the record's.
3. **Tiles as DAG props.** `PropKind::Heightfield` grows a `skirt_depth` and takes the tile's
   spacing and `f64` origin; the mesh is the 257² grid plus its skirt, cooked by `cook_cached`
   with `normal_weight` 0, the grid's edge locked as today, the skirt's bottom free. A tile
   instance joins the scene with its layered material (D-028) and its layer map in the bindless
   set; the pages stream through the pool as any mesh's, roots first. The six level-0 tiles are
   cooked at start behind the loading screen (#25).
4. **The clipmap drives it.** `StreamPlan::around` per viewer each frame; `Residency::update`'s
   `load` list becomes tile jobs at `Low` priority (produce → cook → register, each a job with a
   counter on the last, the cook split per DAG level over `par_*` so no job exceeds a few
   milliseconds), its `unload` list frees the instance and marks the pages evictable. Prefetch:
   the plan is computed a second time at the viewer's position plus `velocity × latency` and its
   new wants queue behind the current plan's. The speed cap of §4 sets `finest` per frame.
5. **Horizon culling and the depth.** The occlusion point per tile (Ring's construction over the
   tile's corner and centre columns at its maximum height) in the instance record; the instance
   cull tests it against the planet's sphere for planet-bound instances before the frustum; the
   plan demotes below-horizon cells. The near plane follows the camera's height above the tile
   under it. `--show-culled` must show no red on any tile the camera can see.
6. **Against pops: the swap criterion, and the island placed on the planet.** A finer tile
   replaces its parent's region only once its pages for the coarsest cut are resident *and* the
   parent's clusters there are at or below 1 px of error (their `self` error projected), so what
   changes is sub-pixel by the cut's own measure; the descent's ꟻLIP per swap (§6) is the test,
   at D-017's thresholds. The island: `island --planet` places the 16 km field on a cell of the
   sphere; its coarse height enters the coarse graph as an uplift override *before* the global
   erosion, so the far tiles already show the island's shape, and tiles inside its footprint
   sample the island's 4 m field instead of amplifying the coarse one, blended over a 1 km band
   at the edge (Kooima's composition, once per tile): the same shape at every level, the detail
   arriving below a pixel.
7. **The demo.** `island --planet` (or `planet`) with a scripted orbit-to-ground descent: orbit
   at 400 km, 10 km over the island, the ground at the coast, golden captures at each, the F1
   overlay's terrain counters, `tools/timings.sh` at the three altitudes, the ꟻLIP log of the
   swaps, and `--radius` exposed (120 km to 6 371 km) so the owner sees the regimes. TAA on.
8. **Then, measured, not assumed:** the CBT spike (the same field as a texture per tile, the LEB
   renderer's scheme in Slang, the same frame) against the DAG per tile; the displacement stage
   for the last octave if the finest level's pages dominate the pool.

**The data model.**

- **`TileRecord`** (CPU, one per produced tile, in the cache and in memory while resident):
  `cell: CellId`; `origin: DVec3` (the cell's centre on the sphere, the tile's `f64` frame);
  `spacing_m: f32` (from the projection at the cell's centre); `samples: 257`; `halo: 8`;
  `height_min, height_max: f32`; `geometric_error_m: f32` (the maximum difference to the parent's
  field, also the skirt's depth); `occlusion_point: DVec3`; `sea_level: f32`;
  `field_digest: u64` (FNV-1a now, BLAKE3-truncated with D-018's container);
  `producer_version: u32`; `props_proxy: Option<..>` (reserved). About 100 bytes.
- **The tile's fields** (in the cache, not in memory once cooked): `height` (273² `f32`, 300 KB),
  `layers` and `weights` (257² `u8` each, 66 KB each), optionally `sediment`, `debris` and `flow`
  at the same size for the placement pass and the water. Half a megabyte a tile before
  compression.
- **The tile's DAG** (the mesh cache, D-025's format): 131 072 triangles plus the skirt's 4 096,
  about 1 060 leaf clusters, about 2 100 clusters in all over 8–9 levels, 27 pages of 128 KiB
  (3.4 MiB) with today's 16-byte vertices; the 112-byte cluster records (235 KB) resident while
  the tile is. Quantised vertices and zstd (D-018) should bring the pages to about a megabyte.
- **The halo** is generation-only: the 8 samples beyond the cell make the border's Horn normals
  and the erosion passes' footprints identical in both tiles sharing an edge, so the locked border
  is exact in geometry *and* shading; it is dropped before the cook.
- **The digest** is of the 273² height field's bits after stage 5 and the layer map's after
  stage 6, in row order; it lives in the record, the CI golden list and D-010's handshake.

**The render-graph passes it adds.** `terrain/upload` on the transfer queue (the arriving tiles'
cluster records and instance rows, beside `streaming/upload`'s pages); `terrain/blas` on the
compute queue, the TLAS refit each frame in the existing `rt/` group; the horizon test inside
`cull/instances`, not a pass; the layer maps as bindless images written by the same upload.
Nothing else: the tiles are instances of meshes, and the culls, the visibility buffer, the layered
shading, the sky, GTAO, the probes and TAA see them as they see the city's ground.

**Expected numbers (orders of magnitude, to be replaced by the demo's).**

| Quantity | Estimate | From |
|---|---|---|
| Coarse genesis | 6 × 1025² (2.3 km): 20 s on four cores; 6 × 2049² (1.15 km): about a minute | the island's 0.33 s a step at 4097² |
| A tile's production | 10–50 ms to amplify 273² (75 k cells, a few dozen local iterations); 0.2 s to cook 135 k triangles | the island's per-cell cost; the city's 12 s for 8 M triangles |
| Tiles wanted a second at 300 m/s | 3–4 across the levels (the finest level's 2.9 km disc turns over in 10 s) | D-037's rings, the coarser levels' geometric series |
| Tiles resident | about 20 a level × 10 levels (1 to the 4.5 m level on a 1 500 km planet): 200, plus the six roots | D-037 |
| Pages in the cut | 3–6 M triangles on screen at 1440p → 30–50 k clusters → 50–85 MiB; a 128–256 MiB pool | the city's 395 pages (49 MiB) for a static view |
| Disk cache | 3.4 MiB a tile today, about 1 MiB compressed; a 16 km island at 4.5 m is 200 tiles | D-025's page size |
| Frame at 1440p, terrain only | 1–2 ms at each altitude (the city's ground and props: 1.05 ms at 1600 × 900; the ballad: 1.63 ms at 1440p) | `docs/PROFILE.md`, `city-blocks.md` |
| Sky from orbit | 0.1 ms with the planet-view table, 0.3 ms with the per-pixel march | D-023, #26 |
| A tile's latency, wanted → resident | 0.3–0.6 s cold (produce, cook, read); tens of milliseconds warm from the cache | the estimates above |

**What to measure** is §6's list, in the demo's exit log and in `docs/demos/island.md`'s planet
section: the three golden captures, the ꟻLIP log of the descent's swaps, the tiles per second and
the pool's turnover, and the digests of the CI cells at one and six workers on both machines.

---

## What the numbers say

Partitions: S2's thirty levels reach a centimetre on Earth and D-037's 27 levels 1.7 cm on a
1 500 km planet; the gnomonic cube's corner cells are about 5× its centre cells and the tangent
warp brings the spread to a few percent; HEALPix is exactly equal-area on rhombi. Clipmaps held a
40 GB heightmap at 100:1 in 2004; CBTs render "planetary scale geometry out of very coarse
meshes" in under 0.2 ms on consoles (2024). Nanite's target is a triangle per pixel; Forge's city
ground is 8 M triangles, 186 k clusters, 14 levels, cooked in 12 s, drawn with a million props at
1.05 ms (1600 × 900), 395 pages (49 MiB) resident in a static view, 540–720 on a 300 m/s flight
uploading 0–1.6 pages a frame. Cesium's tile header is 88 bytes with a bounding sphere and a
horizon occlusion point; its horizon test is two dot products. Space Engineers' planets are
120 km across, Star Citizen's tech is in its fifth version with soils and erosion-derived debris,
Flight Simulator streams more than 2 PB with 400 photogrammetry cities and 1.5 billion
reconstructed buildings, Elite regenerated every landable planet for Odyssey. Forge's estimates:
a 257² tile of 131 k triangles and 27 pages, a quarter of a second to produce, 3–4 a second at
300 m/s, 200 resident, a 128–256 MiB pool, 1–2 ms of terrain at 1440p, a ꟻLIP peak under 0.15
at every swap.

---

## Checked and left out

Kept so the bibliography is auditable: things looked for and not above, with the reason.

- **A "Dupuy & Sperl" adaptive tessellation paper** — none found; the adaptive GPU tessellation
  chapter is Khoury, Dupuy & Riccio (GPU Zen 2, 2019), cited above, and Dupuy's CBT papers
  follow it. Listed so the misattribution is not searched for again.
- **A Google Earth tiling paper** — not found as a primary source; Google's published scheme is
  S2, which is cited; Google Earth's own rendering tiles are not documented publicly.
- **Flight Simulator "Beyond the horizon" talks** — no talk of that title was found; the GDC 2022
  Advanced Graphics Summit session by Lionel Fuentes is the Asobo terrain talk and is cited. A
  Flight Simulator 2024 streaming talk was not found within the budget.
- **KSP2's post's content** — the forum was unreachable and the search record gives only the
  author and the subject; the CBT claim is community discussion and is marked so.
- **Star Citizen's own comm-links and CitizenCon videos** — videos are not citation grade and the
  RSI site was not searched beyond an AMA listing; the wiki stands for both, as in
  `terrain-genesis.md`.
- **Ronchi, Iacono & Paolucci 1996 (the cubed sphere) and Epic's "Large World Coordinates"
  page** — covered in `large-worlds.md`; not re-verified.
- **Terrain shadows from orbit as a published technique** (cascaded shadow maps on a planet) —
  not searched; Forge's shadows are ray queries (D-029) and the entry reasons from the TLAS's
  scale. City lights and airglow for the night side: `lighting-gi.md` §6's night-sky entry.

---

## Verification notes

Checked on 2026-09-26 with WebSearch and WebFetch only, no browser pane. The session's egress
proxy refused every host tried except `github.com` (cesium.com returned "blocked by the network
egress proxy"; the other hosts were not retried after `terrain-genesis.md`'s list of refusals the
day before). Verification therefore has the two grades of issue #99.

- **Fetched and read (GitHub):** CesiumGS/quantized-mesh (the format's README: the header, the
  edge lists, the skirt and horizon sentences quoted), CesiumGS/cesium's `EllipsoidalOccluder.js`
  (the comments quoted), CesiumGS/3d-tiles (the specification's geometric-error and refinement
  sentences), google/s2geometry (the README, which points to the guide), jdupuy's
  LongestEdgeBisection2D and libleb, cuberact's planet demo (the README quoted), Hoimar's
  Planet-Generator, Zylann's solar_system_demo, SebLague's Procedural-Planets, sp4cerat's
  Planet-LOD, TokisanGames' Terrain3D, NVlabs/flip, and, from the day before, fstrugar/CDLOD.
  Quotes from these are verbatim from the pages.
- **Confirmed through the search engine's record of the primary page** (title, authors, venue,
  DOI or date, and the extracts quoted, which are the search engine's extracts of the page named)
  (verified through search results): Zucker & Higashi 2018 (jcgt.org); the S2 guide
  (s2geometry.io); Ring 2013, both posts (cesium.com); Cozzi & Ring 2011; Górski et al. 2005
  (IOPscience, DOI 10.1086/427976); Westerteiger, Gerndt & Hamann 2011 (Dagstuhl OASIcs, the UC
  Davis PDF listing); Kooima et al. 2009 (ACM DL, PubMed, EVL); Lindstrom & Pascucci 2001 (ACM DL,
  LLNL's SOAR page, pascucci.org's PDF listing); Ulrich 2002 (tulrich.com, vterrain.org, flipcode
  2002); Losasso & Hoppe 2004 (hhoppe.com); Clasen & Hege 2006 (EG diglib, DOI
  10.2312/VisSym/EuroVis06/091-098, TIB, the author's page); Dimitrijević & Rančić 2015
  (ScienceDirect, ACM DL) and 2023 (ScienceDirect); Khoury, Dupuy & Riccio 2019 (Semantic
  Scholar, the GPU Zen 2 table of contents); Dupuy 2020 and Benyoub & Dupuy 2024 (ACM DL, arXiv,
  from the day before); Karis 2021 (the Advances 2021 PDF listing); Epic's Nanite Landscape,
  Nanite tessellation (dev.epicgames.com and forum extracts) and Virtual Heightfield Mesh (the
  API page listing, community pages); Reed 2015 and Kemen 2012 (from `large-worlds.md`); Epic's
  World Partition HLOD page (from `large-worlds.md`); Bruneton & Neyret 2008 and Proland's
  documentation (proland.inrialpes.fr, proland.inria.fr, evasion.inrialpes.fr); Outerra 2009 and
  2015; Doc Ross / 80.lv 2018, wccftech 2015, PC Gamer 2021; McKendrick 2017 (GDC Vault 1024265)
  and Murray 2017 (GDC Vault 1024514, gamedeveloper.com); Planet Tech v4, "Terra Firmer" and
  Planet Tech v5 / Genesis (starcitizen.tools); KSP2 Developer Insights #12 and #18
  (forum.kerbalspaceprogram.com listings); Fuentes 2022 (GDC Vault 1027581, 80.lv,
  gamedeveloper.com, the Asobo PDF listing); Rosa 2015 and Petržilka 2015 (blog.marekrosa.org,
  Mod DB); Zechmann & Hlavacs 2025 (arXiv 2510.24764, the University of Vienna's records);
  Hillaire 2020 (from `lighting-gi.md`); Boksanský, Wimmer & Bittner 2019 (SpringerLink, TU Wien,
  the authors' PDF listing); Andersson et al. 2020 (the repository's citation page); UNIGINE's
  Landscape Terrain and double-precision pages; Cesium for Unreal's quickstart and repository
  listing.
- **Weaker confirmations, stated plainly.** Lindstrom & Pascucci's content is described from the
  standard account, not from the abstract. The Clasen & Hege sentence on texture coordinates and
  the Kooima, Dimitrijević, Khoury and Westerteiger sentences are the search engine's extracts of
  the abstracts. The Cesium blog's opening sentence is the search engine's extract. The Star
  Citizen entry rests on a fan wiki, the KSP2 CBT claim on a Steam discussion, the Elite Odyssey
  claim on PC Gamer's headline and summary, the Flight Simulator numbers on press coverage and
  the GDC listing, not on the slides. The ꟻLIP repository now lists the first author as Pontus
  Ebelin; the 2020 paper's byline is Andersson. Ring's horizon test is described from the code's
  structure and comments, not the posts' derivation; the formula in the entry is a paraphrase.
- **Forge's own numbers** (city ground, pages, flight, the island's genesis timings, the sky
  pass) are from `docs/demos/city-blocks.md`, `docs/demos/island.md`, `docs/PROFILE.md`, D-023,
  D-025, D-037, `partition.rs` and `streaming.rs` as of 2026-09-26.
- **Numbers to re-check before they enter a spec:** every estimate in the recommendation's table
  (a tile's production time, the cook's linear scaling from 8 M to 135 k triangles, the pool for
  the cut, the frame at 1440p) is an estimate marked as such, to be replaced by the demo's; the
  5× gnomonic area ratio is the standard figure, not measured here; the "under 0.2 ms" of the CBT
  paper is on console hardware; the Godot demo's constants are its defaults; the 88-byte header
  and the two root tiles are quantized-mesh's defaults.
