# Vegetation and materials

An annotated bibliography for the *visible surface* of Forge's worlds: how trees and plants are
authored, how they are rendered from a metre away to the horizon, how the ground, rocks and buildings
are textured without a large art team, and — added at the owner's request — how one material
definition can drive rendering, physics, audio and gameplay at once. Companion to
[RESEARCH.md](../RESEARCH.md), which already covers procedural terrain (§1), L-systems and shape
grammars (§2), plant ecosystems and GPU placement (§4), procedural noise, stochastic tiling and SDFs
(§5); those entries are referenced here by section and not repeated. Sibling documents:
[large-worlds.md](large-worlds.md) (streaming, virtual textures) and [audio.md](audio.md)
(material-driven sound). Labels follow the house convention — **[paper] [book] [talk] [web] [code]
[docs]** and **foundational / still-current / recent** — and every citation was confirmed against at
least one reachable page; what could not be confirmed is listed at the end, with what was tried.

> **State of the art in five sentences.** Production trees are still grown from a parametric or
> self-organising skeleton (Weber–Penn 1995, space colonisation 2007, RESEARCH.md §4) and dressed with
> scanned bark and leaf atlases; SpeedTree packages exactly that workflow for $19/month, and Unity's
> own tree editor is legacy. Rendering is a four-band ladder — alpha-tested geometry with vertex wind
> near, billboard-cloud leaf cards mid, octahedral impostors far, and a volumetric canopy at the
> horizon — and the newest challenger, Epic's Nanite Foliage (UE 5.7, November 2025, still
> Experimental), replaces the far bands with voxelised aggregates precisely because impostors and LOD
> pops are the weak link. Grass is generated on the GPU per tile from a hash and never stored (Ghost
> of Tsushima 2021), and whole open worlds are shaded through a visibility buffer so that alpha-tested
> foliage is shaded once per pixel (Horizon Forbidden West 2022, Nanite 2021). Environment texturing
> is trim sheets plus tileables plus decals plus vertex paint — a 2015 workflow that is still what
> ships — with stochastic tiling (RESEARCH.md §5) removing the repetition, and CC0 scan libraries
> (Poly Haven, ambientCG) plus Fab's free Megascans slice make the asset side cheap. A single material
> ID looked up per triangle, per terrain texel and per hit is the mechanism every physics engine
> already offers (PhysX, Jolt, Rapier); the engine's job is to make render, physics, audio and weather
> read the same row of the same table.

**Contents**

1. [Tree and plant authoring pipelines](#1-tree-and-plant-authoring-pipelines)
2. [The vegetation rendering ladder](#2-the-vegetation-rendering-ladder)
3. [Grass, ground cover and scatter at scale](#3-grass-ground-cover-and-scatter-at-scale)
4. [Trim sheets and the modern texture workflow](#4-trim-sheets-and-the-modern-texture-workflow)
5. [Materials and texturing without a big art team](#5-materials-and-texturing-without-a-big-art-team)
6. [Rocks, cliffs and ground](#6-rocks-cliffs-and-ground)
7. [Wet, snow and water interaction](#7-wet-snow-and-water-interaction)
8. [Unified physical materials](#8-unified-physical-materials)
9. [Recommendation for Forge](#recommendation-for-forge)
10. [Checked and left out](#checked-and-left-out)
11. [Verification notes](#verification-notes)

---

## 1. Tree and plant authoring pipelines

The owner's question — "use how nature works to build trees, or use models if that makes more sense"
— has a clear answer in the literature and in the tools: everyone grows the *skeleton* procedurally
and nobody grows the *leaves and bark* procedurally. The differences between SpeedTree, Sapling,
Blender add-ons and the research pipelines are differences of parameterisation and UI over the same
two 1990s–2000s models.

**Jason Weber, Joseph Penn. "Creation and Rendering of Realistic Trees." *SIGGRAPH '95*, 1995,
119–128.** [paper] [foundational]
<https://doi.org/10.1145/218380.218427>

The parametric tree model: a trunk and up to four recursive branch levels, each level described by
a few dozen parameters (length and taper ratios, curvature and its variance, branching angle,
down-angle, split count, pruning envelope) plus a leaf distribution, all sampled with per-level
variance so one parameter set yields a species and a seed yields an individual. Arbaro
(<http://arbaro.sourceforge.net/>) states it is an implementation of this algorithm, and the Blender
manual states the same of Sapling.
*Bearing:* the natural *species file* format for Forge — flat, serialisable, seed-driven, thirty
years of tuned examples exist. Use it as the outer parameterisation even if the branches inside are
grown by colonisation.

**Adam Runions, Brendan Lane, Przemyslaw Prusinkiewicz. "Modeling Trees with a Space Colonization
Algorithm." *Eurographics Workshop on Natural Phenomena*, 2007.** [paper] [foundational]
<https://algorithmicbotany.org/papers/colonization.egwnp2007.html>

Extends the authors' leaf-venation model to 3D: scatter attraction points in a crown envelope, let
branch tips grow toward the points that are nearest to them, kill points once reached, repeat. The
parameters map onto things landscape designers recognise — crown shape, density, branch spread — and
the paper is explicit that both overall form and internal structure are controllable. This is the
algorithm the in-house broadleaf trees already use; the 2009 self-organising model in RESEARCH.md §4
is its successor with light competition.
*Bearing:* keep it. Pair it with a Weber–Penn parameter envelope for species control and the
Pałubicki 2009 shadow-propagation model (RESEARCH.md §4) when trees must respond to neighbours.

**Ondřej Šťava, Sören Pirk, Julian Kratt, Baoquan Chen, Radomír Měch, Oliver Deussen, Bedřich Beneš.
"Inverse Procedural Modelling of Trees." *Computer Graphics Forum* 33(6), 2014.** [paper]
[still-current]
<https://doi.org/10.1111/cgf.12282>

Given a polygonal tree (scanned or hand-modelled), estimate the parameters of a stochastic
procedural model that reproduces it, by Markov-chain Monte Carlo over the parameter space with a
similarity metric on branch geometry statistics. The output is a *parameter set*, so the fitted
species can then be re-sampled endlessly.
*Bearing:* the bridge between "use models" and "use nature": fit the generator to a reference or a
scan once, then own the parameters rather than the mesh.

**Bosheng Li, Jacek Kałużny, Jonathan Klein, Dominik L. Michels, Wojtek Pałubicki, Bedrich Benes,
Sören Pirk. "Learning to Reconstruct Botanical Trees from Single Images." *ACM Transactions on
Graphics* 40(6), 2021 (SIGGRAPH Asia).** [paper] [recent]
<https://doi.org/10.1145/3478513.3480525>

Note the title: it is "from single *images*", not "single silhouettes". A network predicts a
species-conditioned radial-depth representation of a tree from one photograph, from which a 3D
skeleton and foliage are reconstructed; the procedural generator supplies the training data.
*Bearing:* the affordable route to "trees that look like *this* region": photograph the reference
forest, reconstruct, fit parameters (Šťava 2014), generate.

**Xiaochen Zhou, Bosheng Li, Bedrich Benes, Songlin Fei, Sören Pirk. "DeepTree: Modeling Trees with
Situated Latents." arXiv:2305.05153, 2023.** [preprint — **not peer-reviewed as fetched**] [recent]
<https://arxiv.org/abs/2305.05153>
**Jae Joong Lee, Bosheng Li, Sara Beery, Jonathan Huang, Songlin Fei, Raymond A. Yeh, Bedrich Benes.
"Tree-D Fusion: Simulation-Ready Tree Dataset from Single Images with Diffusion Priors."
arXiv:2407.10330, 2024.** [preprint] [recent]
<https://arxiv.org/abs/2407.10330>

The credible neural tree work is from the same Purdue/Pirk group, and both papers keep the botanical
model in the loop: DeepTree learns branching as a network queried iteratively from the root, with
"situated latents" that carry environmental state so trees grow around obstacles; Tree-D Fusion
produces 600,000 tree models from street-view photographs by combining diffusion priors with — again
— the space colonisation algorithm for the internal structure. No "TreeGAN" of standing was found
(see *Checked and left out*).
*Bearing:* neither replaces a procedural generator; both are evidence that the generator is the
right substrate and learning is a way to *parameterise* it from data.

**SpeedTree (Unity / IDV). SpeedTree 10 Modeler, Library and Runtime SDK — store and product
pages, 2026.** [docs] [still-current]
<https://store.speedtree.com/> — <https://speedtree.com/>

What it is: a modeler that combines "procedural generators" with "nondestructive manual editing",
shader- and vertex-colour-based wind, dynamic LOD generation, a scanned library ("8k resolution PBR
textures", season variants), export to FBX/OBJ/USD/.ST, and a runtime SDK for PC, console, mobile and
VR. Pricing verified on the store at time of writing: **Indie $19/month** (revenue under $100k),
**Pro $899/year** (under $1M), **Library add-on $999/year**, **Enterprise custom** (over $1M, includes
the runtime SDK, node-locked or floating). Current version 10.2.0. Unity acquired SpeedTree in July
2021.
*Bearing:* at $19/month it is worth having as the *reference implementation* of the pipeline Forge
is building and for hero trees; do not take the runtime SDK dependency — everything it does at
runtime (wind, LOD, billboards) is in this document.

**Epic Games. "Unreal Engine 5.6 is now available" (3 June 2025) and "Unreal Engine 5.7 is now
available" (12 November 2025).** [docs] [recent]
<https://www.unrealengine.com/en-US/news/unreal-engine-5-6-is-now-available> —
<https://www.unrealengine.com/en-US/news/unreal-engine-5-7-is-now-available> — PCG overview:
<https://dev.epicgames.com/documentation/en-us/unreal-engine/procedural-content-generation-overview>

What actually shipped, from the release posts: 5.6 brought PCG node-graph UX, 3D viewport preview,
GPU and multithreaded execution (all Beta) and a **PCG Biome Core v2** plugin (Experimental); the
Witcher 4 tech demo was shown with it. 5.7 declared **PCG production-ready** and introduced **Nanite
Foliage** as *Experimental*, built from **Nanite Voxels** ("millions of tiny, overlapping elements
that read as a solid mass at distance … without cross fades, pops, or the need to author LODs"),
**Nanite Assemblies** (instanced sub-parts to cut memory) and **Nanite Skinning** for wind. PCG itself
is a point-processing node graph: samplers produce points with transforms and attributes, filters
and spawners consume them.
*Bearing:* the authoring model Forge should match is PCG-plus-Biome-Core (rules over points, GPU
executed — the same architecture as Horizon's placement in RESEARCH.md §4); the rendering model to
watch is Nanite Foliage (§2).

**Unity Technologies. "Tree Editor" and "SpeedTree model import", Unity 6 Manual.** [docs]
[still-current]
<https://docs.unity3d.com/6000.0/Documentation/Manual/class-Tree.html> —
<https://docs.unity3d.com/6000.0/Documentation/Manual/SpeedTree.html>

Not dead, but legacy: "For most uses, the SpeedTree Modeler replaces the Tree Editor", and the Tree
Editor "works only with the Built-In Render Pipeline". The SpeedTree import page records two details
worth keeping: SpeedTree trees use "three to four different Materials" (a draw-call cost), and
billboards are *rotated to face the light* during the shadow-caster pass.
*Bearing:* the tree-creator-in-engine idea is abandoned by the one engine that had it; the
light-facing-billboard trick is the cheapest correct impostor shadow (§2).

**Blender: "Sapling Tree Gen" (extensions.blender.org, v0.3.7, GPL-3.0+, Blender 4.4+) and Maxime
Herpin's "Mtree / modular_tree" (GPL-3.0 add-on, MIT library).** [code] [still-current]
<https://extensions.blender.org/add-ons/sapling-tree-gen/> — manual:
<https://docs.blender.org/manual/en/3.6/addons/add_curve/sapling.html> —
<https://github.com/MaximeHerpin/modular_tree>

Sapling is the Weber–Penn model as a Blender curve generator ("The method is presented by Jason
Weber & Joseph Penn"); Mtree is a node-based generator (`TrunkFunction`, `BranchFunction` …) with a
standalone C++ library under MIT. Both are free, both export meshes, neither does wind, LOD or
impostors.
*Bearing:* useful for making *reference* trees and leaf-card bakes offline; Mtree's MIT core is a
readable second implementation to compare against when debugging Forge's own growth code.

**Epic Games. "Fab, Epic's new unified content marketplace, launches today." 22 October 2024.**
[web] [recent]
<https://www.unrealengine.com/en-US/blog/fab-epics-new-unified-content-marketplace-launches-today>

Quixel Megascans moved to Fab: the whole library (17,000+ assets) could be claimed free "for use
with ALL engines" until the end of 2024; from 2025 a curated slice of "more than 1,500 hand-picked
assets" is free to everyone, and the rest is sold under Fab's Standard License in Personal or
Professional tiers (the revenue threshold is on the Fab licence page, not in the post). Scanned bark,
leaf atlases and rocks are what the library is best at.
*Bearing:* check whether the 2024 claim was made; if so, the library is licensed for Forge already.
Either way the 1,500-asset free tier plus CC0 libraries (§5) covers bark, leaves and rocks for a demo.

---

## 2. The vegetation rendering ladder

**Tiago Sousa. "Vegetation Procedural Animation and Shading in Crysis." *GPU Gems 3*, ch. 16,
2007.** [book] [foundational]
<https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-16-vegetation-procedural-animation-and-shading-crysis>

The wind model every engine still ships: *main bending* displaces the whole plant along the wind by
a function of vertex height, *detail bending* moves leaves and branches using vertex-colour channels
for stiffness and phase, and the waves are "four vectorized triangle waves" smoothed with cubic
interpolation instead of sines. Leaf shading approximates subsurface transmission with a thickness
texture and the eye·light term.
*Bearing:* the baseline vertex shader for band 0–1; the vertex-colour phase channel is what the
tree generator must emit alongside positions.

**Sean Feeley. "Interactive Wind and Vegetation in 'God of War'." *GDC 2019*.** [talk] [still-current]
<https://www.gdcvault.com/play/1026036/Interactive-Wind-and-Vegetation-in> — video:
<https://www.youtube.com/watch?v=MKX45_riWQA>

A "dynamic and spatially-varying 3D wind simulation" with "interactive, boneless tree and leaf sway"
and ground-vegetation interaction, using procedural flow, texture flood-filling and fractal noise;
the talk also covers the authoring workflow and the features that failed. Wind is a *field* that
characters, weapons and effects write into and vegetation reads.
*Bearing:* the missing half of Sousa: a world-space wind/interaction field texture that the same
shader samples for bending, trampling and blast. One field feeds trees, grass and cloth.

**Colin Barré-Brisebois, Marc Bouchard. "Approximating Translucency for a Fast, Cheap and Convincing
Subsurface Scattering Look." *GDC 2011*.** [talk] [still-current]
<https://colinbarrebrisebois.com/2011/03/07/gdc-2011-approximating-translucency-for-a-fast-cheap-and-convincing-subsurface-scattering-look/>

Frostbite 2's leaf and thin-object translucency: a baked *local thickness* map plus a view-dependent
back-lighting term, no screen-space blur, a handful of ALU per pixel. It is what "sunlight through
leaves" has meant in real time since.
*Bearing:* the one leaf material for band 0; thickness is trivially baked per leaf card and even
computable from the leaf atlas alpha.

**Xavier Décoret, Frédo Durand, François Sillion, Julie Dorsey. "Billboard Clouds for Extreme Model
Simplification." *SIGGRAPH 2003*.** [paper] [foundational]
<https://maverick.inria.fr/Publications/2003/DDSD03/>
**Stephan Behrendt, Carsten Colditz, Oliver Franzke, Johannes Kopf, Oliver Deussen. "Realistic
Real-Time Rendering of Landscapes Using Billboard Clouds." *Computer Graphics Forum* 24(3)
(Eurographics), 2005.** [paper] [foundational]
<https://doi.org/10.1111/j.1467-8659.2005.00876.x>

Billboard clouds fit a small set of textured, alpha-mapped planes to a model by optimising plane
placement against a coverage error; Behrendt et al. apply it to trees and whole landscapes, fitting
planes to *leaf clusters* so that the mid-distance tree is a few hundred cards instead of tens of
thousands of leaves. This is the "least-squares leaf quads" idea.
*Bearing:* band 1. Generate the cards offline per species archetype (not per instance) from the
grown skeleton; the fit is a one-off cost and the runtime is plain instanced quads.

**Ryan Brucks. "Octahedral Impostors." shaderbits.com, 2018 — and the *ImpostorBaker* Unreal
plugin.** [web + code] [still-current]
<https://shaderbits.com/blog/octahedral-impostors> — <https://github.com/ictusbrucks/ImpostorBaker>

Capture the object from views distributed over an octahedron (or a hemi-octahedron for objects seen
from above the ground plane) into one atlas with depth and normal, then at runtime pick the three
nearest frames and blend them with per-frame parallax from the captured depth, so the impostor holds
up under rotation without popping; depth also drives pixel-depth-offset for self-shadowing. The blog
body is client-rendered and could not be fetched (see *Verification notes*); the plugin (541 stars)
is the reachable artefact.
*Bearing:* still the best *flat* far representation for trees and the one to implement for band 2;
its known limits — no true parallax, no light-facing geometry, nothing for rays — are what the next
three entries and Nanite Foliage exist to address.

**Amplify Creations. "Amplify Impostors" — product page.** [docs] [still-current]
<https://amplify.pt/unity/amplify-impostors/>

The commercial Unity equivalent: spherical and octahedron impostors, in-editor baked, "full
shadowing support", and the honest statement that impostors "are not meant to be used as a 1-1
replacement of your standard geometry" — use the mesh close and the impostor at distance.
*Bearing:* confirms the industry practice: impostors are a band, not a replacement.

**Philippe Decaudin, Fabrice Neyret. "Volumetric Billboards." *Computer Graphics Forum* 28, 2009.**
[paper] [still-current]
<https://maverick.inria.fr/Publications/2009/DN09/>

Store each tree (or leaf cluster) as a small 3D texture and render it as a stack of view-aligned
slices, giving "full parallax effects from any viewing angle" and correct transparency between
overlapping objects without sorting. The slices cost fill rate but the representation is
view-independent, which flat impostors are not.
*Bearing:* the principled fix for the impostor's parallax failure; and the direct ancestor of the
voxel aggregates in Nanite Foliage. For Forge, the crown-as-density-volume idea also gives an
RT-friendly proxy.

**Eric Bruneton, Fabrice Neyret. "Real-time Realistic Rendering and Lighting of Forests." *Computer
Graphics Forum* 31(2) (Eurographics), 2012.** [paper] [still-current]
<https://doi.org/10.1111/j.1467-8659.2012.03016.x>

Forests to the horizon with consistent lighting: near trees as geometry, far trees as a precomputed
view-dependent "z-field" representation, and a lighting model that accounts for tree-to-tree
shadowing, sky occlusion and ground light at forest scale so the transition between representations
does not change brightness.
*Bearing:* the band-3 reference. The in-house "painted canopy" is a special case of this; the paper
supplies what it lacks — a lighting model that matches the near bands so the seam is invisible.

**James McLaren. "Adventures with Deferred Texturing in 'Horizon Forbidden West'." *GDC 2022*.**
[talk] [recent]
<https://www.gdcvault.com/play/1028035/Adventures-with-Deferred-Texturing-in> —
<https://www.guerrilla-games.com/read/adventures-with-deferred-texturing-in-horizon-forbidden-west>

Guerrilla's answer to foliage overdraw: "a visibility buffer drawn as a pre-pass followed by analysis
and shading steps which run entirely in compute shaders", built "primarily for accelerating foliage
and alpha-tested geometry" on PS4/PS5, with software variable-rate shading falling out of the same
pass. Shading cost becomes one evaluation per pixel regardless of how many leaf layers were drawn.
*Bearing:* the single most important architectural decision for the ladder: with a visibility
buffer the near band's overdraw is a *rasterisation* cost only, and Forge's mesh-shader path can
emit visibility IDs directly.

**Epic Games. "Nanite Foliage" — Unreal Engine 5.7 release announcement, November 2025.** [docs]
[recent — Experimental]
<https://www.unrealengine.com/en-US/news/unreal-engine-5-7-is-now-available>

The current challenger to the whole ladder: a voxel aggregate representation for canopies, needles
and grass that is "automatically and efficiently" rasterised at stable frame rates, assemblies for
instanced sub-parts, skinning for wind, and no authored LODs or impostors. Its documentation page
exists but is client-rendered; the announcement is the verifiable statement, and it says
Experimental. The SIGGRAPH 2025 Advances programme has no Epic foliage talk yet.
*Bearing:* do not build Forge's far bands *around* impostors as if they were permanent. Design the
band-2/3 representation as a voxel/density aggregate from the start (Decaudin–Neyret, Bruneton–Neyret)
with the octahedral impostor as the cheap first implementation behind the same interface.

**Nicolas Lopez. "Rendering 'Assassin's Creed Shadows'." *GDC 2025*; Luc Leblanc, Melino Conte. "Ray
Tracing the World of Assassin's Creed Shadows." *SIGGRAPH 2025 Advances in Real-Time Rendering*.**
[talk] [recent]
<https://www.gdcvault.com/play/1035526/Rendering-Assassin-s-Creed-Shadows> —
<https://advances.realtimerendering.com/s2025/index.html>

A shipped open world with "dynamic time of day cycles, a systemic weather system, and seasons" and
ray-traced GI, GPU-driven; the SIGGRAPH talk names "large quantities of dense vegetation" as the
central RT problem. The seasons pipeline (snow accumulation, foliage colour) is in these talks, not in
a public blog post (see *Checked and left out*).
*Bearing:* the closest shipped analogue to Forge's target (open world, seasons, RT). Its lesson for
the ladder: whatever the far representation is, it must be traceable — see the next entry.

**Epic Games. "Hardware Ray Tracing Tips and Tricks." Unreal Engine documentation.** [docs]
[still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/hardware-ray-tracing-tips-and-tricks-in-unreal-engine>

"Geometry with small holes, or lots of little details, can significantly impact performance. For
example, trees and bushes with lots of leaves" — any-hit shaders on alpha-tested leaves are the RT
cost. Impostors are worse, not better: a camera-facing quad has no meaning for a shadow or GI ray
arriving from another direction, and the same is true of a flat card under stereo or very wide FOV,
where the two eyes (or the screen edge) see the frame from angles that differ from the one it was
blended for. No primary source states the VR failure in those words (see *Verification notes*); the
Unity SpeedTree page's need to *rotate billboards toward the light* for shadows is the same defect
seen from the shadow side.
*Bearing:* every far representation needs an RT proxy that is not the raster one: a crown ellipsoid
or coarse voxel density with a leaf-density material for shadow/GI rays, and geometry (not cards)
within the RT cutoff distance.

---

## 3. Grass, ground cover and scatter at scale

Placement rules are in RESEARCH.md §4 (Deussen 1998, Lane 2002, Kapp 2020, Horizon's GPU placement);
this section is what happens *after* a rule says "grass here, density 0.7".

**Eric Wohllaib. "Procedural Grass in 'Ghost of Tsushima'." *GDC 2021, Advanced Graphics Summit*.**
[talk] [recent]
<https://www.gdcvault.com/play/1027214/Advanced-Graphics-Summit-Procedural-Grass> — video:
<https://www.youtube.com/watch?v=Ibe1JBF5i5Y> — companion: Bill Rockenbeck, "Blowing from the West:
Simulating Wind in 'Ghost of Tsushima'", <https://www.gdcvault.com/play/1027350/>

Grass blades are generated on the GPU per terrain tile — position from a hash, shape as a Bézier
curve with per-blade procedural width, tilt and colour — culled and drawn as instanced geometry, and
animated by a world wind field; nothing is stored, so density is a parameter and memory is constant.
The wind talk supplies the field (a fluid-ish simulation on a coarse grid) that the grass, cloth and
particles all read.
*Bearing:* the template for Forge's ground cover: `hash(seed, cell, i)` → blade, exactly the
stateless discipline of RESEARCH.md §3, executed in a compute pass that also does the frustum and
distance cull. Interaction (trampling, cutting) is a small per-tile state texture written by gameplay
and read by the same shader.

**Klemens Jahrmann, Michael Wimmer. "Responsive Real-Time Grass Rendering for General 3D Scenes."
*ACM SIGGRAPH Symposium on Interactive 3D Graphics and Games (I3D)*, 2017.** [paper] [still-current]
<https://www.cg.tuwien.ac.at/research/publications/2017/JAHRMANN-2017-RRTG/>

Each blade is a geometric object with a physics response to gravity, wind and object collisions,
placed on arbitrary surfaces (not just heightfields), with culling and an adaptive pipeline that keep
it real-time. The collision response is the part Tsushima's talk treats lightly.
*Bearing:* the reference for bending and trampling as a per-blade spring rather than a painted
displacement; use its model in the near ring only.

**Ulrich Haar, Sebastian Aaltonen. "GPU-Driven Rendering Pipelines." *SIGGRAPH 2015 Advances in
Real-Time Rendering*.** [talk] [foundational for the technique]
<https://advances.realtimerendering.com/s2015/index.html> — slides:
<https://advances.realtimerendering.com/s2015/aaltonenhaar_siggraph2015_combined_final_footer_220dpi.pdf>

The pipeline every scatter system now assumes: all instances in GPU buffers, per-instance and
per-cluster culling in compute, "per-material instance batching", indirect draws, and "hundreds of
thousands of independent objects" at 60 fps on 2015 consoles. Scatter at Forge's scale is this plus
a cell hash: instances live in world-space cells, cells are culled first, instances second.
*Bearing:* the mesh-shader path makes cluster culling a shader stage rather than a compute pass, but
the data layout (cells → instance ranges → clusters) is the same and should be designed once for
trees, rocks, grass and props.

---

## 4. Trim sheets and the modern texture workflow

The owner asks whether trim sheets are still in use and how professionals use them. Yes, and the
workflow has not changed in shape since 2015: *tileables* for broad surfaces, *trim sheets* for
edges, borders and architectural detail, *decals* for damage and story, *vertex paint* for blending
and dirt, and a *material layering* system to combine them. Unique unwraps are reserved for hero
props; megatexture-style unique texturing is a streaming question (large-worlds.md), not an authoring
one.

**Polycount Wiki. "Texture atlas."** [web — community reference] [still-current]
<http://wiki.polycount.com/wiki/Texture_atlas>

The community's definition and index: an atlas is "the method of packing many separate textures
together into a single texture" — of which a trim sheet is the special case where the atlas is
organised as horizontal strips of tiling detail (mouldings, beams, edges, panel lines) that models
map onto by sliding UV shells along the strip. The page lists the tutorials the practice grew from
(Klevestav's modular sci-fi walls, Wanlass's modular buildings, Mathis on texture-space efficiency)
and covers gutters and edge padding. The dedicated "Trim sheet" page refused connections at time of
writing.
*Bearing:* the vocabulary and the constraints (padding for mips, strip heights as powers of two)
that a *procedural* trim generator must respect.

**Morten Olsen. "The Ultimate Trim: Texturing Techniques of Sunset Overdrive." *GDC 2015*.** [talk]
[foundational for the workflow]
<https://www.gdcvault.com/play/1022324/The-Ultimate-Trim-Texturing-Techniques>

The talk that standardised the technique: one fixed trim layout and normal-map structure shared
across the whole game, a UV script that snaps shells onto the strips, and a shader that adds surface
variation so the repetition is not read. The session text is explicit that this let "a relatively
lean environment art team" build a large open world.
*Bearing:* the exact organisational move for a medieval village: **one regional trim layout**,
authored (or generated) once per material family, with the building grammar (RESEARCH.md §2) doing
the UV snapping instead of a Maya script.

**Sébastien Deguy, Rogelio Olguin, Brad Smith. "Texturing Uncharted 4: a matter of Substance." *GDC
2016* (sponsored session).** [talk] [still-current]
<https://www.gdcvault.com/play/1023488/Texturing-Uncharted-4-a-matter>

Naughty Dog's account of moving material creation to procedural Substance graphs — tileables,
trims and layered materials generated from parameters, with the PBR transition and content management
for a AAA scope. Sponsored, so read for workflow not for claims.
*Bearing:* evidence that the highest-end linear game of its generation textured with procedural
tileables plus layering, not with unique unwraps.

**Romain Lemaire. "Helldivers 2: Diving into Biome Material Creation with a Tiny Team." *GDC 2025*
(Adobe Developer Summit, sponsored).** [talk] [recent]
<https://www.gdcvault.com/play/1035421/Adobe-Developer-Summit-Helldivers-2>

The sole material artist on a procedural-planet game describes "the evolution of the material
pipeline" and the "procedural tools and efficient workflow strategies" that let one person cover
every biome. Sponsored again, but the premise — one artist, many biomes, procedural materials — is
Forge's situation exactly.
*Bearing:* the existence proof for §5's thesis; watch it before staffing the material side.

**Epic Games. "Using Material Layers in Unreal Engine." Unreal Engine 5.8 documentation.** [docs]
[still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/using-material-layers-in-unreal-engine>

A *Material Layer* asset defines a surface (its tileable and parameters), a *Material Layer Blend*
defines a mask (vertex colour, height-lerp, world-space projection), and instances stack them in the
instance editor without touching the graph. This is the engine-side form of the layering workflow.
*Bearing:* Forge's material definition should be a small stack of `(layer, blend)` pairs evaluated
in the visibility-buffer material pass — which is also the natural place to put trim, decal and
weather layers.

**R. Steven Glanville. "Texture Bombing." *GPU Gems*, ch. 20, 2004.** [book] [foundational]
<https://developer.nvidia.com/gpugems/gpugems/part-iii-materials/chapter-20-texture-bombing>

Divide UV space into cells, stamp a randomly chosen and transformed image into each, sample the
neighbouring cells and resolve overlaps by priority. RESEARCH.md §7 recorded this as an unverified
lead; it is verified here. The stochastic-tiling family in RESEARCH.md §5 (Heitz–Neyret, Deliot–Heitz,
Burley, Mikkelsen hex-tiling) is the modern, histogram-correct successor for *continuous* textures;
bombing remains the right tool for *discrete* features — moss patches, lichen, stains, bolts.
*Bearing:* bombing decals over a hex-tiled base is how a village wall stops looking tiled at three
metres.

**Natalya Tatarchuk. "Dynamic Parallax Occlusion Mapping with Approximate Soft Shadows." *I3D 2006*.**
[paper] [foundational]
<https://doi.org/10.1145/1111411.1111423>

Ray-march a heightfield in tangent space per pixel to find the displaced surface, with LOD toward
plain normal mapping and approximate self-shadowing. Still the standard for cobbles, brick and stone
walls where silhouettes do not matter.
*Bearing:* POM on the tileable, *not* on the trim, is the cheap depth that sells a stone village at
walking distance; with a visibility buffer it runs once per pixel.

---

## 5. Materials and texturing without a big art team

**Adobe. "Substance 3D Designer" product page.** [docs] [still-current]
<https://www.adobe.com/products/substance3d/apps/designer.html>

Node-based procedural material authoring with parametric `.sbsar` export; sold only inside the
Substance 3D Collection, **US$59.99/month** individual, $119.99/month per team seat, at time of
writing. The fitting-to-photo research (MATch, node-graph optimisation) is in RESEARCH.md §5.
*Bearing:* optional. One seat for the person who owns the material library pays for itself the first
time a bark or plaster needs re-tuning; not a runtime dependency.

**Rodolphe Suescun et al. "Material Maker." MIT, built on Godot.** [code] [still-current]
<https://github.com/RodZill4/material-maker> — <https://www.materialmaker.org/>

A free node-based procedural texture and 3D-painting tool, node graphs exportable as shader code,
Windows/macOS/Linux builds. Less mature than Designer but sufficient for tileables, trims and masks —
and, being MIT, its node library is a readable reference for a Slang port of the same generators.
*Bearing:* the zero-cost default; the *generators* (noise, Voronoi, bevel, tile, blend) are the same
ones Forge will run at load time in Slang for procedural trims.

**Poly Haven — licence page; ambientCG — licence page.** [web] [still-current]
<https://polyhaven.com/license> — <https://docs.ambientcg.com/license/>

Both are **CC0**: "anyone can then use the work in any way and for any purpose, including commercial
purposes" (Poly Haven); "copy, modify, distribute and perform the assets, even for commercial
purposes, all without asking permission", including shipping raw files in a game (ambientCG). Scanned
bark, ground, plaster, stone, roof tiles, plus HDRIs.
*Bearing:* the base library. Everything shipped from these can be redistributed, forked and
regenerated without a licence check — which matters when materials are procedurally *derived* from
scans at runtime.

**Morten S. Mikkelsen. "Surface Gradient–Based Bump Mapping Framework." *Journal of Computer
Graphics Techniques* 9(3), 2020.** [paper] [still-current]
<https://jcgt.org/published/0009/03/04/> — reference implementation (MIT):
<https://github.com/mmikk/surfgrad-bump-standalone-demo>

RESEARCH.md §7 recorded this as a lead verified only from an index page; it is confirmed here from
the journal page and the author's repository. The framework composes bump contributions from any
number of sources — tangent-space normal maps, height maps, object-space normals, **triplanar
projection**, **decal projectors** and **procedural 3D noise** — as surface gradients, so they add
correctly regardless of UV space.
*Bearing:* the one normal-compositing rule for Forge: terrain triplanar + tileable detail normal +
trim normal + decal + weather (snow, wetness) all become gradients summed in the material pass.

**Microsoft. "BC7 Format", DirectX 11 documentation.** [docs] [still-current]
<https://learn.microsoft.com/en-us/windows/win32/direct3d11/bc7-format>

Sixteen bytes per 4×4 block (8 bits/pixel) with eight block modes, subsets and rotation, "high-quality
compression of RGB and RGBA data" with bit-exact decoders. BC5 (two-channel, same rate) is the normal
map format; BC4 (one channel, 4 bpp) is for masks, heights and thickness.
*Bearing:* desktop Vulkan targets: BC7 for albedo/ORM, BC5 for normals, BC4 for scalar layers;
every stochastic-tiling scheme must be validated against BC block artefacts (Deliot–Heitz, RESEARCH.md
§5, has the notes).

**Khronos. "KTX File Format Specification 2.0" (rev. 4, 2025-02-20); Binomial. "Basis Universal"
(v2.5, Apache-2.0); Arm. "astc-encoder" (Apache-2.0).** [docs + code] [still-current]
<https://registry.khronos.org/KTX/specs/2.0/ktxspec.v2.html> —
<https://github.com/BinomialLLC/basis_universal> — <https://github.com/ARM-software/astc-encoder>

KTX2 is the container: textures identified by `VkFormat`, mip chains, arrays and cube maps, with
supercompression (BasisLZ, Zstandard, ZLIB) and streaming-friendly layout. Basis Universal supplies
the two transcodable formats — ETC1S (small, fast) and UASTC (8 bpp, high quality) — that decode to
BC7/BC5/ASTC/ETC on the target; its README notes (March 2026) that an XUASTC embedding in KTX2 is
being standardised with Khronos. ASTC spans 0.89–8 bpp and is the mobile default; on desktop it is
not the choice.
*Bearing:* store everything as KTX2 + Zstd with BC7/BC5/BC4 payloads for the desktop build; keep
UASTC as the *source* encoding only if a mobile or web target ever appears. The virtual-texture page
cache in large-worlds.md should be the same format so the tile loader is one code path.

**Christopher A. Burns, Warren A. Hunt. "The Visibility Buffer: A Cache-Friendly Approach to Deferred
Shading." *Journal of Computer Graphics Techniques* 2(2), 2013.** [paper] [foundational]
<https://jcgt.org/published/0002/02/04/>

Store only a triangle ID and instance ID per sample (4 bytes) instead of a G-buffer, then reconstruct
attributes and shade in a later pass; bandwidth drops and shading happens once per pixel. The paper
that Nanite and Horizon Forbidden West (§2) both build on.
*Bearing:* the material implication the owner asked about: **materials must be indexable by ID and
evaluable from a compute or full-screen pass** — no per-draw uniform state, bindless textures, one
material table in a storage buffer.

**Brian Karis, Rune Stubbe, Graham Wihlidal. "A Deep Dive into Nanite Virtualized Geometry."
*SIGGRAPH 2021 Advances in Real-Time Rendering*.** [talk] [recent]
<https://advances.realtimerendering.com/s2021/index.html> — slides (16 MB):
<https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf>

The production form of the visibility buffer: cluster rasterisation to a 64-bit vis-buffer, a
*material ID* pass that writes each pixel's material as depth, then one full-screen (tile-culled) pass
per material that passes the depth test only where its ID matches, reconstructing barycentrics and
derivatives analytically.
*Bearing:* the material pass Forge should implement: material ID → depth trick makes "hundreds of
materials, one pass each" cheap on the hardware depth test, and the same pass evaluates trims,
layers, decals and weather (§8) per pixel.

---

## 6. Rocks, cliffs and ground

**Adrien Peytavie, Eric Galin, Jérôme Grosjean, Stéphane Mérillou. "Procedural Generation of Rock
Piles using Aperiodic Tiling." *Computer Graphics Forum* 28(7) (Pacific Graphics), 2009,
1801–1809.** [paper] [still-current]
<https://doi.org/10.1111/j.1467-8659.2009.01557.x>

Rock piles, scree and stone walls generated by aperiodic tiling of precomputed tiles of packed,
non-intersecting rocks, with implicit-surface rocks whose shapes are eroded procedurally; large
piles are locally evaluable without global simulation, in the spirit of RESEARCH.md §3.
*Bearing:* scree slopes, fallen walls and river cobbles from a tile set and a hash; the implicit rock
primitives are the same SDF vocabulary as RESEARCH.md §5 and Paris et al. 2019's implicit terrain
features (RESEARCH.md §1).

**Jeremy Moore. "Terrain Rendering in 'Far Cry 5'." *GDC 2018*.** [talk] [still-current]
<https://www.gdcvault.com/play/1025480/Terrain-Rendering-in-Far-Cry>

The GPU compute pipeline "used for LODing, culling, stitching and rendering the height field
terrain", and how it integrates with "procedurally generated cliffs and displacement geometry" from
the world-generation pipeline (Carrier, RESEARCH.md §2). Cliffs are meshes placed by rule where slope
exceeds a threshold, shaded with tiling rock and trims, and blended into the heightfield.
*Bearing:* the shipped answer to "cliffs via tiling + trim": cliff meshes are scatter, not terrain,
and the seam is hidden by blending (next entry) rather than by geometry.

**Epic Games. "Runtime Virtual Texturing." Unreal Engine 5.8 documentation.** [docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/runtime-virtual-texturing-in-unreal-engine>

An RVT is a GPU-generated shading cache over a large area — landscape material, decals and splines
rendered into it on demand — and static meshes can *write into* it so a rock's base and the terrain
share one texture, which is what makes mesh-to-terrain blending seamless; the rock's own material then
samples the RVT near the ground using its height above terrain as the blend.
*Bearing:* the cheap version of the blend is a depth/height-based lerp between the rock material and
a terrain sample at the rock's footprint; with Forge's terrain already virtual-textured
(large-worlds.md) the RVT form is nearly free. Moss, snow and wetness are the same top-down layer
(§7) applied to rocks and terrain alike, composited as surface gradients (Mikkelsen, §5).

---

## 7. Wet, snow and water interaction

Water rendering is elsewhere; this is what water *does to materials*.

**Sébastien Lagarde. "Water drop 2b – Dynamic rain and its effects" (3 January 2013) and "Water
drop 3a – Physically based wet surfaces" (19 March 2013).** [web — engineering write-ups]
[still-current]
<https://seblagarde.wordpress.com/2013/01/03/water-drop-2b-dynamic-rain-and-its-effects/> —
<https://seblagarde.wordpress.com/2013/03/19/water-drop-3a-physically-based-wet-surfaces/>

The series behind Remember Me's rain, still the standard reference: wet surfaces darken because
water fills the pores ("water replacing the air have an index of refraction higher") and become more
specular because a thin film sits on top, modelled as a porosity-driven albedo darkening plus a
smoothness increase and, physically, a dual-layer BRDF; puddles from a painted or height-derived
flood level with procedurally generated ripple normals; and a console budget for all of it.
*Bearing:* the weather layer in Forge's material pass: a per-material *porosity* scalar and a world
*wetness* field are enough to derive the darkening and gloss; puddles come from the terrain's own
flow accumulation (RESEARCH.md §1) rather than from painting.

**Colin Barré-Brisebois. "Deformable Snow Rendering in Batman: Arkham Origins." *GDC 2014*.** [talk]
[still-current]
<https://www.gdcvault.com/play/1021004/Deformable-Snow-Rendering-in-Batman> — author's post
compiling the GDC and GTC versions:
<https://colinbarrebrisebois.com/2014/08/16/deformable-snow-and-directx-11-in-batman-arkham-origins/>

Snow as a displacement heightfield that characters and objects carve into by rendering their
footprint from below into a deformation texture, then tessellating the snow surface with it; the
edges get the raised rim that sells the depth.
*Bearing:* the mechanism for trails in snow, sand and mud is identical — one deformation texture per
material class, written by anything that touches the ground.

**Anton Kai Michels, Peter Sikachev. "Deferred Snow Deformation in Rise of the Tomb Raider." In *GPU
Pro 7*, 2016.** [book chapter — practitioner] [still-current]
<https://doi.org/10.1201/b21261-5>

The deferred form of the same idea: deformers write into a persistent snow-height buffer in a
separate pass, the terrain reads it, and snow *fills back* over time, giving trails that persist and
fade without per-object state.
*Bearing:* the version to implement — it composes with GPU-driven scatter, needs no CPU knowledge of
who stepped where, and its "refill rate" is a material parameter (§8).

---

## 8. Unified physical materials

The owner's requirement: "if I see a brick wall I expect brick-like friction, solidity, sound. Sand,
wood, snow, etc. If I walk on an ice lake I expect to slip just because it's ice — behaviour built
into the engine." And the same material must *remember* contact: "foot or paw traces in snow and
sand, and soft things" — persistent footprints, paw prints and wheel tracks written by physics into
a displacement layer and faded by weather. Every engine below already has half of this — a physics
material with a surface type looked up from a hit. None ships the other half, where the *same* row
also owns the render layers, the audio set and the deformation response and is mutated by weather.
The sources establish the lookup and deformation mechanisms; the Recommendation proposes the table.

**Epic Games. "Physical Materials in Unreal Engine." Unreal Engine 5.8 documentation.** [docs]
[still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/physical-materials-in-unreal-engine>

"Physical Materials are used to define the response of a physical object when interacting
dynamically with the world" — assets "applied to physically simulated primitives, directly or via
materials". The overview links a User Guide, a Reference (friction, restitution, density, surface
type, combine modes) and a *Physical Material Mask* page (a mask texture that lets one render material
return different physical materials per texel); those three sub-pages are client-rendered and could
not be read (see *Verification notes*). The pattern is nonetheless the industry's: a `SurfaceType`
enum in project settings, a physical material per render material, per-collision-shape overrides, and
a hit result that carries the physical material so footstep, decal and damage code branch on it.
*Bearing:* Unreal's model is the minimum; its weakness for Forge is that the render material and the
physical material are two assets joined by a slot rather than one definition.

**Unity Technologies. "Physics Material asset reference." Unity 6 Manual.** [docs] [still-current]
<https://docs.unity3d.com/6000.0/Documentation/Manual/class-PhysicsMaterial.html>

Dynamic friction, static friction, bounciness (0 "like soft clay"), and *combine modes* for friction
and bounce (default Average); assigned per collider. No surface type — games add their own
enum alongside.
*Bearing:* the friction/restitution/combine triple is the physics core of the unified row; the
combine rule is what makes "ice against rubber boot" a table lookup.

**Jorrit Rouwé. Jolt Physics — `PhysicsMaterial` and `MeshShapeSettings` class reference; repository
README (MIT).** [docs + code] [still-current]
<https://github.com/jrouwe/JoltPhysics> — <https://jrouwe.github.io/JoltPhysics/class_physics_material.html>
— <https://jrouwe.github.io/JoltPhysics/class_mesh_shape_settings.html>

Jolt — "used by Horizon Forbidden West and Death Stranding 2" — models a material as a base class
that "describes the surface of (part of) a shape", meant to be subclassed with game data, and states
the purpose directly: "The 2 materials involved in a contact could be used to decide which sound or
particle effects to play." Mesh shapes carry `mMaterials`, and each triangle names its material with
`mMaterialIndex`; heightfields do the same per sample.
*Bearing:* the per-triangle material index is the join key. If Forge's mesh format stores one
`MaterialId` per triangle and hands the same index to the physics mesh, the render vis-buffer and the
collision hit resolve to the same row by construction.

**NVIDIA. PhysX 5 SDK Guide — "Geometry" (triangle meshes and height fields).** [docs] [still-current]
<https://nvidia-omniverse.github.io/PhysX/physx/5.6.0/docs/Geometry.html>

"Like height fields, triangle meshes support per-triangle material indices", supplied to the cooking
library; height-field samples carry "two materials (for the two triangles in the samples rectangle)",
and if none is given "the material of the PxShape instance is used".
*Bearing:* confirms the same convention in the other major engine; the terrain's *dominant layer at a
point* maps naturally onto a per-sample material index, which is how open-world games get the
footstep surface from the splat map without a texture read on the CPU.

**Dimforge. Rapier user guide — "Colliders" (friction and restitution).** [docs] [still-current]
<https://rapier.rs/docs/user_guides/rust/colliders>

Rust's mainstream rigid-body engine: friction and restitution per collider with a
`CoefficientCombineRule` of Average, Min, Multiply or Max, with precedence Max > Multiply > Min >
Average when two colliders disagree. No material object and no per-triangle materials — a trimesh
collider has one coefficient pair.
*Bearing:* if Forge uses Rapier, the per-triangle material must live in Forge's own side table keyed
by the hit's feature/triangle index; if it uses Jolt through bindings, it is native. Either way the
combine rule should be a property of the material row, not of the collider.

**Firelight Technologies. FMOD Studio manual — "Parameters Reference" (2.03).** [docs] [still-current]
<https://www.fmod.com/docs/2.03/studio/parameters-reference.html>

Labeled user parameters "use strings (labels) instead of numerical values", each label indexed
automatically, so a footstep or impact event selects its variation from a `surface` parameter set by
game code. This is the "switch on surface type" pattern; Wwise's equivalent (Switch Containers keyed
by a Switch such as ground material) is the same design, but every Audiokinetic documentation host
refused automated fetching (see *Checked and left out*).
*Bearing:* audio needs one string or index per material row — the *footstep set* and *impact set* —
and, if audio.md adopts modal synthesis, a stiffness/damping pair for synthesised impacts. Nothing
about the material should be authored twice.

### Deformation traces: footprints, paw prints, wheel tracks

The verified technique lineage is three sources, two of them in §7. **Barré-Brisebois, GDC 2014**
(Batman: Arkham Origins): objects that touch the snow render their footprint into a deformation
heightmap, the snow surface is tessellated against it, and the displaced rim gives the print its
depth read. **Michels & Sikachev, *GPU Pro 7*, 2016** (Rise of the Tomb Raider): the same idea made
*deferred* and *persistent* — deformers write into a world-anchored snow-height buffer in their own
pass, the terrain and every snow-covered mesh read it, and the buffer *refills* over time so trails
fade on their own. The third is the general terrain form:

**Egor Yusov. "Real-Time Deformable Terrain Rendering with DirectX 11." In *GPU Pro 3*, 2012.**
[book chapter — practitioner] [still-current]
<https://doi.org/10.1201/b11642-4>

A GPU-resident terrain heightfield that is modified at runtime — craters, tracks, digging — and
rendered through hardware tessellation with adaptive LOD, with the modification applied as a
displacement to the base terrain rather than a rewrite of it. It is the deformable-*ground* case
(mud, sand, ash) as opposed to the snow-*layer* case, and the two compose: the ground deforms slowly
and permanently, the layer on top deforms instantly and refills.
*Bearing:* Forge's deformation is one clip-mapped RG16 layer around the player (depth, and
age-or-material for the fade), written by a compute pass fed from **physics contact manifolds** —
feet, paws, wheels, fallen bodies — each stamp scaled by the contacting material's `max_depth` and
the contact's pressure. Rendering reads it as a displacement in the mesh-shader terrain path (or as
parallax where tessellation is not worth it), composites the rim as a surface gradient (Mikkelsen,
§5), and *weather* is what erases it: the refill rate is a row property (dry sand refills fast,
wet mud slowly, snow at the snowfall rate). Which games shipped which variant beyond these three —
Red Dead Redemption 2, Horizon Forbidden West's sand and snow, the Spintires/MudRunner/SnowRunner
mud — could not be sourced (see *Checked and left out*), so the design below leans on the three
citations, not on those titles.

*Also load-bearing for this section:* Lagarde's wet-surface model (§7) and the Assassin's Creed
Shadows seasons-and-weather pipeline (§2) — the shipped cases of weather mutating a material's look;
whether they also mutated friction or sound is not public, which is why §8's data model below is a
proposal rather than a citation.

---

## Recommendation for Forge

*Opinion, shaped by the project's constraints — small team, heavy procgen, Vulkan/Slang with mesh
shaders and RT — rather than by citation count.*

**Tree pipeline: grow, don't model.** Grow every species from a Weber–Penn parameter file whose
branches are produced by space colonisation with Pałubicki-style shadow competition (RESEARCH.md §4);
per-instance variation is the seed, per-site adaptation is Pirk 2012's skeleton deformation. Author
nothing by hand except the *materials*: bark tileables and leaf/needle atlases from Fab's free
Megascans slice and CC0 scans, with thickness baked per leaf card. When a species must match a real
region, photograph it and fit the parameters (Li 2021 → Šťava 2014). Buy one SpeedTree Indie seat
($19/month) as the yardstick and for the three hero trees a demo needs; never link its runtime.
Leaf cards (band 1) are billboard-cloud fits computed offline per species archetype, not per
instance. This is the combination the owner asked about, and it is the professional choice for a team
that cannot staff a foliage artist: SpeedTree's own value is that it is this pipeline with a GUI.

**Rendering ladder, four bands** (numbers assume a 1.8 m eye height, 90° horizontal FOV, 1440p; tune
by screen-space size, not metres):

| Band | Distance | Representation | Wind | Shadow / RT |
|---|---|---|---|---|
| 0 | 0–40 m | full skeleton mesh + individual leaf quads, alpha-tested, two-sided, mesh-shader clusters into the visibility buffer | Sousa main+detail bending driven by Feeley's world wind field; per-blade springs for grass | geometry in shadow maps and BLAS |
| 1 | 40–150 m | trunk + primary branches + 200–400 billboard-cloud leaf cards (Behrendt 2005) | main bending only | cards in shadow maps; a coarse crown mesh in BLAS |
| 2 | 150–600 m | hemi-octahedral impostor, 3-frame blend, depth-based PDO (Brucks) — behind an interface that a voxel aggregate (Decaudin–Neyret / Nanite-Voxel style) can replace | phase-shifted atlas UV wobble | impostor rotated to face the light in the shadow pass (Unity SpeedTree trick); RT sees a crown ellipsoid with a leaf-density material |
| 3 | 600 m–horizon | canopy volume: forest density field lit à la Bruneton–Neyret — the existing "painted canopy", promoted from a texture to a lit heightfield with per-cell density and species colour | none (colour noise) | canopy heightfield casts a single far cascade; RT ignores it |

Cost model, in one line: **per-pixel cost = layers rasterised × alpha-test + 1 shade**, and the
visibility buffer is what makes the "+ 1 shade" true. Band 0 is where overdraw lives — a 25 m oak
filling half the screen at 15 m is easily 8–15 alpha-tested leaf layers per pixel — so the near band's
budget is rasteriser throughput, and the tools are: front-to-back instance order, a leaf-cluster
depth pre-cull in the mesh shader, and never shading in the raster pass. Bands 2–3 are cheap by
construction (one quad, three taps; one heightfield); their risk is *correctness* — parallax, stereo,
rays — not cost, which is why each has an RT proxy in the table and why the interface must admit a
voxel aggregate later. Decide the RT cutoff (probably the band 1/2 boundary) and make everything
beyond it a proxy.

**Grass and scatter.** Tsushima's model, unchanged: blades generated per tile in compute from
`hash(seed, cell, i)`, Bézier shape, density from the placement rules of RESEARCH.md §4, world wind
field shared with trees, one RG8 interaction texture per tile written by characters, wheels and
blades (bend direction + cut flag) and a snow/mud deformation buffer (Michels–Sikachev) beside it.
Trees, rocks and props share one GPU-driven scatter layout — cells → instance ranges → clusters —
culled in the mesh-shader task stage.

**Materials and the village.** Yes to trim sheets, and this is how: a *regional material set* is
**2–4 trim sheets** (dressed stone, timber framing, plaster/daub, roofing), **6–10 tileables**
(rubble wall, cobble, plaster, planks, thatch, tile, mud, moss), **one decal atlas** (cracks, stains,
soot, lichen, moss patches, bombed over the base) and **vertex-paint masks** for dirt, damp and wear.
The building grammar (Wonka 2003 / Müller 2006, RESEARCH.md §2) emits *semantic tags* on faces —
cornice, sill, lintel, beam, jamb, quoin — and a UV pass snaps each tagged strip onto the matching trim
row exactly as Olsen's Sunset Overdrive script did, so ten thousand different houses share four
textures. Regional identity is a *different trim set with the same layout*: swap the atlas, keep the
grammar. Because the team is small, generate the trims: mouldings and beams are SDF profiles extruded
along a strip (RESEARCH.md §5, Quilez) with Worley/Perlin weathering and a bombed damage layer,
rendered once to KTX2 at load or at build; only the tileables need scans. Hex-tiling (RESEARCH.md §5)
on the tileables and bombing on the decals remove repetition; POM on the tileables adds depth.

**Texture and shading pipeline.** KTX2 + Zstd, BC7/BC5/BC4, virtual-textured via large-worlds.md.
Visibility buffer everywhere (Burns–Hunt, Karis), one material table in a storage buffer, bindless
textures, material-ID-as-depth per-material passes; a material is `(layers[], blends[], trim?,
decals?, weather response)` evaluated once per pixel, with all normal contributions summed as surface
gradients (Mikkelsen 2020).

**The unified material table.** One row per material, one `MaterialId(u16)` referenced by the
vis-buffer material pass, by every triangle of every mesh (render and collision share the index),
by every terrain texel (dominant layer → id), by decals (override id) and by the audio and gameplay
systems through the hit result. A sketch:

```rust
struct Material {
    // render
    layers: [LayerRef; 4], blends: [BlendRef; 3], trim: Option<TrimRef>, decal_set: Option<DecalSetRef>,
    // physics
    friction_static: f32, friction_dynamic: f32, restitution: f32, combine: CombineRule,
    density: f32, solidity: f32 /* 0 = fluid/penetrable … 1 = rigid */,
    // audio
    footstep_set: SoundSetId, impact_set: SoundSetId, absorption: f32,
    modal: Option<(stiffness: f32, damping: f32)>,          // if audio.md takes modal synthesis
    // gameplay
    tags: MaterialTags /* SLIPPERY | SINKABLE | FLAMMABLE | CLIMBABLE | DEFORMABLE | … */,
    sink_depth: f32, move_speed_scale: f32,
    // deformation traces (footprints, paw prints, wheel tracks) — None = rigid
    deform: Option<Deform { max_depth: f32, rim_height: f32, min_pressure: f32, refill_rate: f32 /* m/s, weather-scaled */ }>,
    // weather response — how this material changes state
    porosity: f32,                                          // wet darkening (Lagarde)
    wet: StateOverride, frozen: StateOverride, snow: StateOverride,
}
struct StateOverride { render_layer: Option<LayerRef>, friction_scale: f32, footstep_set: Option<SoundSetId>, tags_add: MaterialTags, tags_remove: MaterialTags }
```

Runtime: the weather system owns three low-resolution world fields — *wetness*, *temperature* and
*snow depth* (plus the deformation buffer) — sampled at the hit point or the pixel. The *effective*
material is `base ⊕ override(state)` where state is derived from the fields by thresholds stored on
the row: a `water` row whose temperature field has been below 0 °C for the row's `freeze_hours`
becomes `ice` — the collider material index is swapped, friction drops to the override, the
`SLIPPERY` tag appears and the character controller (which only reads tags and friction) slips with
no ice-specific code; a `stone_path` row at wetness 0.8 darkens by porosity, raises gloss, and
switches its footstep set to the wet variant; a `soil` row under 30 cm of snow renders the snow
layer on top (surface-gradient composite), reads `sink_depth` from the snow override and slows
movement. Contact manifolds from the physics step are the *only* writer of the deformation layer:
each contact whose material row has `deform` and whose pressure exceeds `min_pressure` stamps a
print of `max_depth` with a `rim_height` rim — a boot, a wolf's paw, a cart wheel, a body — and the
weather system decays the layer at that row's `refill_rate` scaled by snowfall or rain, so prints in
dry sand vanish in seconds, in mud over hours, in snow when it snows again. Rendering, physics, audio
and gameplay never see "ice" or "footprint"; they see one row, one state and one layer.

**The demo that proves it.** A 4 km × 4 km temperate forest with a river valley and a sixty-building
village on one slope, at 120 fps at 1440p on a current desktop GPU: ~1.2 M trees (≈20 k in bands
0–1, the rest impostor/canopy), grass to 150 m, three trim sheets and eight tileables for the whole
village, every surface a row in the material table. Metrics on screen: rasterised layers per pixel
(target < 6 average in the near band), shading passes per frame, draw count, VRAM for textures.
Walk from the village square (stone: hard footsteps) across a muddy field (sink, slow), into the
forest (leaf litter), out onto the frozen lake (slip) while rain turns to snow and the ground darkens,
then whitens, with no code path that names a material.

---

## Checked and left out

Kept, as in RESEARCH.md, so the bibliography is auditable.

- **Ryan Brucks' "Octahedral Impostors" blog body** — the page is client-rendered; direct fetch and a
  reader proxy both returned only tags and a 2023 timestamp. Cited above via the reachable
  ImpostorBaker repository; the technique summary is from the plugin and from general knowledge, not
  from the post's text.
- **xraxra's "IMP" Unity impostor implementation** — the repository no longer exists under that
  account (404; the account's current repositories are unrelated). Not cited.
- **Amplify Impostors manual page** (limitations, hemi-octahedron) — 403; only the product page is
  cited.
- **Wwise "Understanding Switches"** — three Audiokinetic hosts (en/library, library, public-library
  2024.1.4) returned 403 or a CAPTCHA page. The FMOD labeled-parameter page carries the pattern.
- **Unreal documentation sub-pages** — Physical Materials *User Guide*, *Reference* and *Physical
  Material Mask*; *Landscape Materials* (physical-material output); *Nanite Foliage*; *Mesh Decals*;
  the 5.6 and 5.7 *release notes* — all returned only a table of contents (client-rendered), directly
  and via proxy. The overview page, the release *announcements*, PCG, RVT, Material Layers and HW RT
  pages rendered and are cited.
- **Enshrouded (Keen Games) vegetation/voxel tech** — no talk found; GDC Vault keyword browsing for
  2024 and 2025 returned member-only or empty listings and no search was possible. Nothing public
  could be confirmed.
- **Trim sheets in Doom Eternal, Star Citizen, Halo, Dishonored 2, Alien: Isolation** — GDC Vault
  keyword browsing found only "Building Fear in Alien: Isolation" (2015), AI and level-design talks
  for Dishonored 2 (2017), nothing for Doom or Star Citizen in the years tried, and a paywalled
  listing for Halo. The verified shipped-game trim references are Sunset Overdrive (2015) and
  Uncharted 4 (2016); Helldivers 2 (2025) is the verified small-team procedural-material case.
- **Tim Simpson / Polygon Academy trim tutorials** — polygonacademy.com is suspended; **Leonardo
  Iezzi** — domain does not resolve; **Jonas Rönnegård** — Gumroad page returned no product text.
  The Polycount atlas page is the verified tutorial index instead.
- **Polycount "Trim sheet" wiki page** — connection refused at both attempts; the "Texture atlas"
  page (via proxy) is cited.
- **Textures.com licence** — the licence and FAQ pages returned only a tagline; terms unverified.
- **"Tree Vegetation" Blender add-on** — Blender Market redirected to Superhive and the product
  page did not resolve to a product; not confirmed.
- **"TreeGAN" / a credible 2023–2025 GAN tree model** — none found; *TreeMeshGPT* (CVPR 2025) turned
  up but is about mesh generation with a token *tree*, not botanical trees, and is not cited.
- **A Guerrilla SIGGRAPH 2022 talk on rendering vegetation in Horizon Forbidden West** — does not
  exist as such; the 2022 Advances course had only "Rendering Water in Horizon Forbidden West"
  (Malan). The vegetation-relevant talk is McLaren's GDC 2022 deferred texturing, cited in §2.
- **A Far Cry 5/6 forest or vegetation talk** — none; the 2018 Far Cry 5 talks are procedural
  world generation (RESEARCH.md §2), terrain rendering (§6 here), water and the asset build system.
- **Assassin's Creed Shadows seasons blog post (Ubisoft)** — the news hub 404s; the two 2025 talks
  are cited instead.
- **Red Dead Redemption 2 snow/mud trails, Assassin's Creed III snow, Horizon Forbidden West sand and
  snow deformation** — no primary technical source located (Vault keyword browsing empty or
  unrelated for 2013 "snow", 2018 "red dead", 2019 and 2022 "deformation", 2023 "sand"; the 2022–23
  Horizon talks are water, superstorms, deferred texturing, tools and machines). Batman: Arkham
  Origins (GDC 2014) and Rise of the Tomb Raider (*GPU Pro 7*) carry the technique.
- **A Rise of the Tomb Raider GDC 2016 snow talk** — not found (2016 "snow" and "tomb raider"
  browsing returned narrative and audio sessions only); the *GPU Pro 7* chapter is the citation.
- **Spintires / MudRunner / SnowRunner deformable mud** — no talk or paper found (Vault browsing for
  "mudrunner" 2018 and "snowrunner" 2021 returned empty listings; no search was available). Yusov's
  *GPU Pro 3* chapter is the cited general deformable-terrain reference instead.
- **A Frostbite "material system" or material-database talk** — nothing verifiable found without
  search; not cited.
- **Substance 3D plans page** — two pricing URLs 404; the price above is from the Designer product
  page.
- **Karis et al. 2021 slides** — the PDF is 16 MB and exceeded the fetch limit; verified from the
  course index page.

---

## Verification notes

- **Method.** The session's web-search budget was already exhausted when this task began, so
  *every* citation was verified by fetching a known URL with the fetch tool only — **no browser
  pane was opened at any point**. Sources: publisher DOI pages via the Crossref API
  (Weber & Penn, Šťava, Li, Behrendt, Bruneton & Neyret, Peytavie, Tatarchuk, Michels & Sikachev,
  Yusov),
  arXiv abstract pages and the arXiv API (DeepTree, Tree-D Fusion), author pages (algorithmicbotany,
  maverick.inria.fr, Lagarde, Barré-Brisebois, Mikkelsen's repository), GitHub, Khronos, Microsoft,
  vendor documentation, GDC Vault browse/play pages (session descriptions are public; videos are
  members-only), and YouTube's oEmbed JSON endpoint for video titles and channels — YouTube watch
  pages themselves were never loaded; each video-backed talk (Ghost of Tsushima grass, God of War
  wind) is also confirmed by its GDC Vault session page.
- **Blocked hosts and workarounds.** `web.archive.org` is refused by the fetch tool outright, so
  archive fallbacks were unavailable. HAL (Anubis challenge), `store.speedtree.com`,
  `unrealengine.com` news, Adobe pricing and Audiokinetic returned 403 to direct fetches; the
  SpeedTree store, the Epic announcements and the Polycount wiki were read through a public
  reader proxy (`r.jina.ai`) and the facts quoted are from those renders. YouTube watch pages are
  blocked but oEmbed is not.
- **Corrections carried.** Li et al. 2021 is "from Single *Images*", not "Silhouettes". Nanite
  Foliage shipped in **5.7 (November 2025)** as Experimental, not in 5.6; 5.6 shipped PCG Biome Core
  v2 (Experimental) and the Witcher 4 demo. Unity's Tree Editor is *legacy* (Built-in RP only), not
  removed. Mikkelsen's surface-gradient paper and Glanville's texture bombing, both leads in
  RESEARCH.md §7, are now verified and promoted.
- **Claims made without a primary source.** That flat impostors fail under stereo/wide-FOV viewing
  and under ray tracing is argued in §2 from three verified facts (Decaudin–Neyret's stated
  motivation, Unity's light-facing billboard shadow pass, Epic's RT guidance on leafy geometry) and
  from geometry; no page stating the VR failure directly was found. The distance bands, overdraw
  figures and demo numbers in the Recommendation are estimates, not measurements.
- **Dates.** Checked 23 September 2026. Prices (SpeedTree, Substance) and the Fab free tier are as
  displayed that day and will drift.
