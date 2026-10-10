# Research — People: one face up close, a city's crowd at a glance, and the clothes, motion, talk and memory between them

> Companion to `assets-and-licences.md` §2–§3 (MetaHuman usable in any engine as class 2, its DNA and
> RigLogic libraries MIT since June 2026; MPFB, Anny, MHR and Rocketbox as class 1; "build the human
> runtime first, a generator later"), `population-simulation.md` (the tiers: T3 records for everyone,
> T2 coarse agents, T1 full agents ~100–150 per player within ~150 m, T0 the crowd drawn on the client
> as the real population; §3 personalities, appointments and the cheap relationship tables; §5.3
> 10–20 KB/s per client), `procedural-content.md` §3.7 (garment programs), `animation.md` (clips,
> inertialization, motion matching, IK, powered ragdolls, GPU Gems 3 and Hitman crowds), 
> `game-ai-ecosystems.md` (§1 Ruskin's fact-matched barks and smart objects with reservations, §3
> crowds and the simulation LOD), `smart-systems.md` §3 (the Animation Budget Allocator, Mass's LOD),
> D-010 (netcode), D-012 and its amendment (`forge-anim`), D-016 (determinism), D-021 (the visibility
> buffer), D-029 (sun shadows by ray query), D-052 (dual quaternions), #165, #169 (morph targets,
> eight influences), #168 (skinned meshes at scale) and #225 (language models later). Written
> 2026-10-10 for the owner's question of that day: people as close as possible to Unreal, Gray Zone
> Warfare, Star Citizen and GTA 6 quality, with varied clothes, starting simple; each person with a
> home, an occupation, appointments and a personality; scripted dialogue first; first person by
> default; visemes much later; a friendly tone with varied ages, bodies and clothes and no
> exploitation of women; a million residents with hundreds to thousands visible at once. The
> motivating example is a city sandbox, whose own sizing is in an appendix kept with it. Sources were
> checked with WebFetch, WebSearch and `gh` (GitHub's API) only, no browser pane, nothing downloaded;
> each entry says whether it was read or only seen as a search record ([Verification
> notes](#verification-notes)). Numbers marked *(estimate)* are this note's arithmetic.

The short answer: **the people a player looks at and the people a player sees are two different
systems, joined by one identity.** Up close, a person is a MetaHuman-class asset: a DNA file whose
behaviour layer RigLogic evaluates (three small sparse kernels, now MIT), a skeleton of up to 713 face
joints and 669 blend shapes at the top LOD, screen-space subsurface skin, a refractive eye, hair cards
and fitted clothes, animated by clips, IK and later motion matching. Epic's own specification shows how
fast that collapses: by its LOD 3 the head has 2 500 vertices and 283 joints, by LOD 7 130 vertices and
26 joints, and the body stops at 1 507 vertices. Beyond about 15 m a face is under 20 pixels tall
*(estimate)*, and beyond about 150 m a whole person is under 15. So Forge needs a skeletal path for at
most the ~150 T1 people near a player, a baked path (vertex- or bone-animation textures on merged crowd
meshes, shadows from pre-built poses) for the next few thousand, and impostor cards to the 1.5 km draw
radius. Unreal 5.8's MetaHuman Crowds does exactly this swap (actors near, instanced skinned meshes
far, Mass in between) and claims thousands on high-end hardware; Hitman's crowds of 1 200–1 700 per
level, the Matrix Awakens' 35 000 simulated pedestrians and Watch Dogs: Legion's few dozen scanned
faces blended into thousands of heads set the scale. Variety is cheap where eyes go first (head, hair, upper body,
colour; McDonnell's perception studies) and expensive where they do not; a million people need a
million *seeds*, a few dozen face archetypes, a few hundred garments and a colour system, not a
million assets. Clothes follow body shape best as per-garment morph targets bound offline to the body
(MakeHuman's long-standing proxy method), which also steers clear of Roblox's cage patent and EA's
deformation-map patent. Conversation is staged like The Witcher 3's systemic dialogues (lines with ids,
a generator for cameras, idles and gestures), carried as line ids over the network, with language
models as a later, recorded input. Relationships are a few bytes per pair that ever met; memory of a
player is a small record per (person, player). The one live patent to discuss before building gossip
is Warner Bros.' Nemesis family (to 2036).

---

## Contents

1. [The human runtime](#1-the-human-runtime)
2. [The authoring pipeline and how much variety a million people need](#2-the-authoring-pipeline-and-how-much-variety-a-million-people-need)
3. [Clothing at scale](#3-clothing-at-scale)
4. [Animation](#4-animation)
5. [Conversation, dialogue and the in-world device](#5-conversation-dialogue-and-the-in-world-device)
6. [Relationships and memory](#6-relationships-and-memory)
7. [Netcode for people near players](#7-netcode-for-people-near-players)
8. [What professional engines do](#what-professional-engines-do)
9. [Recommendation for Forge](#recommendation-for-forge)
10. [What to measure](#what-to-measure)
11. [Patents](#patents)
12. [Checked and left out](#checked-and-left-out)
13. [Verification notes](#verification-notes)

---

## 1. The human runtime

What Forge has today (from the repository): `forge-anim` reads glTF skins and clips and samples them
deterministically (D-012's amendment, #165), plays them through inertialized switches, a blend space,
two-bone IK and a look-at (#167); `forge-render`'s skin pass skins four or eight joints a vertex
(#169) by linear blending or, per mesh, dual quaternions (D-052), adds sparse morph targets per cluster
before the joints (#169), writes the previous positions for the motion vectors, and refits each mesh's
BLAS in place. Skinned meshes are cooked as one level of roots bounded by the sphere of every pose
(#168 wants per-frame bounds, a skinned LOD and vertex-animation textures). What follows is what a
MetaHuman-class person adds to that.

### 1.1 MetaHuman DNA and RigLogic

**Epic Games, OpenRigLogic: README, `docs/design/design.md` ("RigLogic 4"), `include/dna/layers`,
`examples/Basic.cpp`.** [repo] (read through `gh api`; licence MIT, created 5 June 2026, last pushed
30 September 2026)
<https://github.com/EpicGames/openriglogic>

The repository holds the DNA library (reading and writing MetaHuman DNA) and RigLogic (real-time
evaluation of the MetaHuman rig), C++11 with CMake and optional Python bindings through SWIG; a
`5.8` branch is the stable one, `main` experimental; build options turn on an AVX joint path and
half-float joint storage (F16C). Creator's **DCC Export** gives one DNA file for the head and one for
the body, plus textures; grooms and clothing are not part of that package, the README says.
The design document describes the evaluation: inputs are rig controls, outputs are joint
transformations, blend-shape weights and animated mask multipliers (the wrinkle maps), computed in
three stages: pose-space correctives (products of control values), a linear stage that is an ordinary
sparse matrix-vector product, and conditionals (piecewise-linear chains, under 3 % of the data).
Version 4 stores the linear matrix as dense sub-matrices per joint group in blocks of four (SSE
width), padded at about 2.5 % memory cost and nearly six times faster than the old compressed rows;
LODs are vertical slices of the same matrix, so switching LOD is a matter of moving bounds, and a
*max-LOD* chosen at load skips the rest. The DNA layers in the headers are the
Header, Descriptor, Definition, Behavior and Geometry, plus RBF, twist-swing, machine-learned and joint
metadata behaviours. The Geometry reader exposes per-mesh positions, UVs and normals as separate
arrays, OBJ-like vertex layouts, faces, skin weights with a maximum influence count per mesh, and blend
targets as sparse deltas with vertex indices. The example's whole loop is `setRawControl`,
`calculate`, then reading joint, blend-shape and animated-map outputs.
*Bearing:* small enough to port. The evaluation is a product loop, a block-sparse matrix-vector product
and a clamp chain, the kind of code `forge-anim` already writes without transcendentals (D-016 does
not bind presentation, but the same discipline costs nothing). The DNA binary format is defined by MIT
code. A Rust port of the reader (Behavior and Geometry layers only) and of the evaluation avoids the
C++ and CMake step D-012's amendment refused for ozz; the C++ library, built once outside the
workspace, is the test oracle (a download, so the owner's go first).

**Epic Games, MetaHuman-DNA-Calibration, `docs/dna.md`.** [repo] (read through `gh api`; Epic's own
MetaHuman DNA Calibration licence agreement, not MIT; its README says it does not support characters
made in 5.6 or later)
<https://github.com/EpicGames/MetaHuman-DNA-Calibration>

The clearest description of the layers: the Descriptor holds name, age, facial archetype and
compatibility parameters for mixing DNA files; the Definition holds names, the mapping of joints,
blend shapes, animated maps and meshes to each LOD, the joint hierarchy and bind pose; the Behavior
maps GUI controls to raw controls and computes correctives, joints, blend-shape and animated-map
weights; the Geometry holds meshes, skin weights and blend-shape deltas. Behavior and Geometry do not
depend on each other, so a runtime can load only what drives the rig.
*Bearing:* Forge loads Behavior for every face it animates and Geometry once per archetype at cook
time; a crowd face needs neither (§1.5). The calibration tools themselves are not for Forge (licence,
version).

**Epic Games, "MetaHuman DNA", "Editing modes", "Controlling MetaHuman levels of detail" and "Platform
support and LOD specifications for MetaHumans".** [docs] (all read)
<https://dev.epicgames.com/documentation/metahuman/metahuman-dna-rig-definition-and-rig-operation> ·
<https://dev.epicgames.com/documentation/metahuman/editing-modes> ·
<https://dev.epicgames.com/documentation/metahuman/controlling-metahuman-levels-of-detail-lods-in-unreal-engine> ·
<https://dev.epicgames.com/documentation/metahuman/platform-support-and-lod-specifications-for-metahumans>

RigLogic maps hundreds of named channels to joints, to per-vertex displacement at LOD 0 only and to
wrinkle triggers; there are eight LOD sets, one joint set spanning them, lower LODs dropping joints by
exclusion. Skin influences are capped at 12 joints for LODs 0–2, 8 for 3–5 and 4 for 6–7. Above the
neck (face, grooms) up to 8 LODs, below it (body, legs, feet) 4; one body change for every two face
changes; forcing LOD 0 or 1 across a crowd is called out as a performance risk. The specification
table (a guide, not a target):

| LOD | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|---|
| Head vertices | 24 000 | 12 000 | 6 000 | 2 500 | 1 300 | 560 | 270 | 130 |
| Head joints | 713 | 529 | 397 | 283 | 84 | 70 | 41 | 26 |
| Head blend shapes | 669 | – | – | – | – | – | – | – |
| Head skin influences | 12 | 12 | 12 | 8 | 8 | 8 | 4 | 4 |
| Animated (wrinkle) maps | yes | yes | no | no | no | no | no | no |
| Body vertices | 30 500 | 7 600 | 3 350 | 1 507 | – | – | – | – |
| Body correctives | yes | yes | no | no | – | – | – | – |
| Hair strands | 50 000 | 25 000 | – | – | – | – | – | – |
| Hair card vertices | 30 000 | 15 000 | 10 000 | 3 000 | 1 500 | – | – | – |
| Hair mesh vertices | – | – | – | – | – | 500 | 250 | 100 |
| Physics | yes | yes | yes | no | no | no | no | no |

Mobile is recommended at LOD 3 and 2 048-texel textures; PC at LOD 0 and up to 8 192.
*Bearing:* the numbers for §1.5's ladder. Two things to change in Forge's skin pass: twelve influences
for the top three head LODs (or prune to eight at import and measure the error), and blend shapes only
at the top LOD, where #169's sparse morphs already fit.

**Ana Kessler, "Epic Revealed the Tech Side of MetaHuman Creator Face Rigs", 80.lv, 10 May 2022.**
[web] (read; the whitepaper it links, `rig-logic-whitepaper-v2`, is a PDF over WebFetch's size limit)
<https://80.lv/articles/epic-revealed-the-tech-side-of-metahuman-creator-face-rigs/>

RigLogic came from 3Lateral (part of Epic), builds on FACS expressions and Maya rig values, solves at
30 frames a second or more, and its innovations are pruning extraneous joint deformations, reducing
joint sets across LODs at no cost, and smart storage of correctives; the DNA's
geometry layer is separate, so one DNA applies to characters of different shapes; all MetaHumans share
one face rig.
*Bearing:* one rig for every face means one control set (the same as ARKit-style animation curves, §5.5)
drives every person; per-face cost is the evaluation at that face's LOD, which no source publishes
(What to measure).

### 1.2 Skeletons and skinning at scale (#168)

**Epic Games, "Nanite Virtualized Geometry" (5.8 documentation).** [docs] (read)
<https://dev.epicgames.com/documentation/unreal-engine/nanite-virtualized-geometry-in-unreal-engine>

Nanite skeletal meshes draw a whole mesh in one call, cast virtual shadow maps and can be instanced
with animation banks; they have no geometry LODs, only animation LODs, and morph targets are not
supported.

**Epic Developer Community, "Status of Skinned Meshes (and Metahuman) and Nanite Support", 21 May – 31
August 2026.** [web] (read)
<https://forums.unrealengine.com/t/status-of-skinned-meshes-and-metahuman-and-nanite-support/2835575>

Asked what blocks MetaHumans on Nanite (morphs, grooms, cloth, translucency as of 5.5), an Epic
developer answered on 31 August 2026 that little had changed, that 5.8 has experimental Nanite
translucency, and that work was starting with progress hoped for by the first quarter of 2027.
*Bearing:* the engine that invented cluster-DAG rendering does not yet run its own faces through it.
Forge's DAG for skinned meshes (#168 step 3) is worth it for bodies and clothes; a face with 669 blend
shapes and per-LOD joint sets is better served by the DNA's own eight LODs.

**Epic Developer Community, "AnimBank and Instanced Skinned Mesh Component", 8–10 June 2026.** [web]
(read)
<https://forums.unrealengine.com/t/animbank-and-instanced-skinned-mesh-component/2734825>

For Hitman-style crowds, the answer from Epic (Euan Carmichael's posts speak for the company) is Mass
with instanced skinned meshes (ISKM) and an `AnimSequenceTransformProvider` once 5.8 ships, with a
warning that these are newer and less proven than the vertex-animation-texture (VAT) and Mass
pipeline; instanced skinned meshes run no animation blueprints, so the coming crowd sample moves
people between dehydrated Mass entities on instanced meshes and hydrated actors with skeletal meshes,
synchronising playback between them; the Animation Sharing plugin (leader poses) is much slower than
the instanced approach.
*Bearing:* the industry's split, stated plainly: near people are actors with full animation graphs,
far people are instances reading shared animation data, and the hand-off syncs the clip and its time.
Forge's T1 → T2 → T0 hand-off (population note §1.3) gets the same rule for animation: the clip id and
phase cross the boundary.

**NVIDIA, `vk_animated_clusters` (nvpro-samples).** [repo] (read through `gh api`; Apache-2.0, pushed
30 September 2026)
<https://github.com/nvpro-samples/vk_animated_clusters>

Ray tracing animated meshes through `VK_NV_cluster_acceleration_structure` (part of RTX Mega
Geometry): clusters of up to ~128 triangles (64 triangles and 64 vertices worked well) are built, not updated, but
*cluster templates* built once let each animated instance's clusters be instantiated fast; NVIDIA
recommends them over triangle-BLAS refits for skinned animation, while static
content keeps triangle BLASes, which trace faster.
*Bearing:* the eventual answer to "a BLAS per skinned person" on NVIDIA; vendor-only, so under
Forge's cross-vendor rule it can only be an optional path beside refits (and beside §1.5's baked
poses).

**What Forge should do for #168, by this note's reading:**
- **Bounds:** the skin pass writes each cluster's sphere per frame (#168 step 2), which the clusters'
  sparse morphs already require.
- **Skinned LOD:** cut the DAG in the bind pose with each cluster's error bound inflated by the largest
  displacement its bones can cause (a per-cluster constant from the clip set), then skin only the
  vertices of the selected clusters. Skinning the whole DAG would pay for vertices never drawn. For a
  MetaHuman the DNA's own LODs come first (their joint sets shrink with them); the DAG matters for
  bodies and garments past LOD 1.
- **Who skins where:** the compute skin pass stays for people whose BLAS is refitted (the nearest few
  dozen); crowd instances skin in the mesh shader from bone textures and never write vertices (§1.5).

### 1.3 Skin, eyes, teeth and tongue

**Jorge Jimenez et al., "Separable Subsurface Scattering", *Computer Graphics Forum* 2015; project page
and `iryoku/separable-sss`.** [paper] [repo] (page read; repository read through `gh api`)
<https://www.iryoku.com/separable-sss/> · <https://github.com/iryoku/separable-sss>

Two separable models approximate a 2D diffuse reflectance profile with two 1D convolutions, run as
post-processing in under half a millisecond a frame with seven samples per pixel, using importance
sampling and jitter. The 2011 code carries a two-clause BSD-style licence whose binary clause asks
distributions to reproduce, in their documentation, a short notice crediting Jimenez and Gutierrez.
*Bearing:* the near-skin method for a visibility buffer (D-021): shading writes diffuse and specular
apart for pixels of the skin class, a two-pass blur of the diffuse along the profile follows, then the
recombine; TAA cleans its jitter. The attribution line goes in `CREDITS.md`.

**Epic Games, "Subsurface Profile Shading Model" and "Shading Models".** [docs] (both read)
<https://dev.epicgames.com/documentation/en-us/unreal-engine/subsurface-profile-shading-model-in-unreal-engine> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/shading-models-in-unreal-engine>

Unreal's skin scattering works in screen space: a separable filter by default, Burley's normalised
profile on request (more accurate, and it needs TAA); full-resolution diffuse and specular need a
64-bit scene colour, else a checkerboard split; light skin's visible scatter distance is given as about
1.2 cm; at distance scattering looks like diffuse light, so the base colour must already match; known
artefacts are banding at large radii and a grey outline where non-skin occludes skin; no backscatter.
The pre-integrated skin model trades accuracy for a lower cost; the Eye model exposes iris mask and
distance and is flagged as hard to author by hand.
*Bearing:* the LOD for skin is free in screen space (a far face covers few pixels), but the base colour
must be authored to match the scattered look, and thin parts (ears, nostrils) need a transmission term
the screen-space blur lacks.

**Eric Penner, George Borshukov, "Pre-Integrated Skin Shading", *GPU Pro 2* (2011), and Penner's
SIGGRAPH 2011 "Advances in Real-Time Rendering" slides.** [book] [talk] (search records)
<https://www.slideshare.net/slideshow/penner-preintegrated-skin-rendering-siggraph-2011-advances-in-realtime-rendering-course/13966747>

A pixel shader with a lookup table of scattering against curvature and the light angle, with no blur
pass.
*Bearing:* the mid-distance skin (faces 20–60 px tall) and the fallback if the screen-space pass costs
too much on crowds. The 2011 publication is the prior art against Tencent's 2019 patent on a
*simulated* pre-integration function (Patents).

**NVIDIA, RTX Character Rendering SDK (`NVIDIA-RTX/RTXCR`).** [repo] (read through `gh api`; "NVIDIA
RTX SDKs LICENSE", the same family as NRD's; pushed 11 September 2026)
<https://github.com/NVIDIA-RTX/RTXCR>

Path-traced hair (Chiang's near-field BSDF and an analytical far-field model) on linear swept spheres
with a triangle-strip fallback (DOTS), and skin that extends Burley's diffusion profile with a single-
scattering term for forward transmission; runs on RTX 20 and newer, Radeon RX 7000 and newer, Intel
Arc.
*Bearing:* a reference implementation of skin and hair for a path tracer; royalty-free with notices
like NRD, which Forge already ships. Not the raster path.

**RoboPoets, "Digital Humans for Godot Engine" (`RoboPoets/digital_human`).** [repo] (read through
`gh api`; MIT code, CC0 art; last pushed 31 December 2023)
<https://github.com/RoboPoets/digital_human>

An eye shader that makes refraction and depth in the shader alone, on a single mesh unwrapped with
the iris at the UV centre, with limbus width and darkness, pupil size, parallax occlusion and SSS
(caustics not done); a skin shader driven by albedo, normal, specular, subsurface and transmission
maps, with roughness derived from the normal map so animated wrinkle normals change it, tattoo and
make-up layers (make-up above the scattering), and animated wrinkles still to do.
*Bearing:* a class 1 reference for Forge's eye class and skin inputs, small enough to read in an
afternoon.

**Teeth and tongue.** No source found describes MetaHuman's mouth shading; Epic's DNA page says only
that the head mesh is the skin-covered surface and that other meshes cover parts of the eyes and mouth
(search record of the page read above), and Unreal's API fits teeth (mesh 1) and the eyes (3
and 4) from the DNA. A design from first principles, for Forge:
- teeth and gums as a skin-class material with a short scatter radius and a wet specular;
- the mouth interior darkened by an occlusion term that follows the jaw's opening (the jaw joint's
  angle from the rig), or by ray-traced ambient occlusion where available, since screen-space AO
  cannot see inside a mouth;
- the tongue skinned to the face rig's tongue joints, which Audio2Face also drives (§5.5);
- a thin wet layer along the lower eyelid and lips (MetaHuman's separate eye-edge and saliva meshes,
  by their names in community tools; not confirmed in a source read).
Teeth and tongue matter within about 3 m, where a mouth is 20 px wide or more *(estimate)*.

### 1.4 Hair: cards against strands

**Epic Games, "Hair Rendering and Simulation".** [docs] (read; a short overview page)
<https://dev.epicgames.com/documentation/en-us/unreal-engine/hair-rendering-and-simulation-in-unreal-engine>

Grooms simulate and render hundreds of thousands of hairs in real time, with cards managed by the same
system. The MetaHuman specification above gives the split: strands at LODs 0–1 (50 000 and 25 000),
cards from LOD 0 to 4 (30 000 to 1 500 vertices), meshes from LOD 5 (500 to 100); the LOD page's
example treats LOD 3 as the best card LOD, and one console variable replaces strands by cards.

**Jon Valdes, Robin Taillandier and the Frostbite team, "Frostbite Hair Rendering and Simulation, Part
2", EA.** [web] (read through the page's AMP copy; Part 1 and the "future of hair" post returned 404)
<https://ea.com/frostbite/amp/news/frostbite-hair-rendering-and-simulation-2>

Strands are simulated and rendered one by one, about 10 000 in the demo asset; NURBS are tessellated to
about 25 points per strand for long hair and 3–5 for short; cards are cheap to compute but slow
and hard to author well. TechRadar's headline (the article body truncated) gives 50 000 strands per
character in Dragon Age: The Veilguard.
<https://www.techradar.com/gaming/consoles-pc/dragon-age-the-veilguards-hair-technology-features-50-000-individual-strands-per-character>

**David Hart, Pawel Kozlowski, "Render Path-Traced Hair in Real Time with NVIDIA GeForce RTX 50 Series
GPUs", NVIDIA Technical Blog, 6 February 2025; Khronos, `VK_NV_ray_tracing_linear_swept_spheres`.**
[web] [docs] (blog read; the extension's reference page seen as a search record)
<https://developer.nvidia.com/blog/render-path-traced-hair-in-real-time-with-nvidia-geforce-rtx-50-series-gpus> ·
<https://docs.vulkan.org/refpages/latest/refpages/source/VK_NV_ray_tracing_linear_swept_spheres.html>

The linear swept sphere is a thick, round line segment whose radius varies along it, one vertex per
strand segment on average; animated hair traces about twice as fast as with the triangle fallback and
needs about 5× less
memory; hardware intersection on RTX 50, a software path on older GPUs. The Vulkan extension is
NVIDIA's own, not ratified.
*Bearing (§1.4):* **cards for everyone first**, shaded with a Marschner-style hair model and dithered
or alpha-to-coverage edges that TAA resolves (the owner sees shimmer at once, so cards are judged in
motion). Strands are a later hero feature for the one to four people at a table, and the LSS path an
NVIDIA-only option. MetaHuman's grooms leave Creator as Unreal assets; no documented export path out of
Unreal was found (Checked and left out), so cards may have to be exported as meshes or rebuilt.

### 1.5 LODs from hero to crowd

**Rachel McDonnell, Micheál Larkin, Simon Dobbyn, Steven Collins, Carol O'Sullivan, "Clone Attack!
Perception of Crowd Variety", *ACM Transactions on Graphics* 27(3), 2008, DOI
10.1145/1360612.1360625.** [paper] (record and abstract read at Goldsmiths' repository)
<https://research.gold.ac.uk/id/eprint/15203>

In perception experiments, "clones of appearance are far easier to detect than motion clones";
colour variation, random orientation and motion mask cloned models; a cloned motion is noticed
whatever model performs it; proximity, model type and characteristic
motion change the thresholds.
*Bearing:* far people can share clips freely with phase offsets (Animation Sharing, VAT), but not
appearance. Variety money goes into how people look, not into how many clips the crowd has.

**McDonnell, Larkin, Hernández, Rudomin, O'Sullivan, "Eye-catching Crowds: Saliency based selective
variation", *ACM TOG* 28(3), 2009, DOI 10.1145/1531326.1531361.** [paper] (search record; Trinity's and
eDeposit Ireland's pages refused the fetch)

Eye tracking shows viewers look at the head and upper torso first and most, whatever the orientation,
motion, sex, age or clothing; colour variation restricted to salient parts reads as varied as full
variation; head accessories, top texture and face texture work about equally well, facial geometry
less.
*Bearing:* the variety budget, ranked: hair and headwear, the top garment's colour and pattern, face
texture and tone, then face shape.

**Ladislav Kavan, Simon Dobbyn, Steven Collins, Jiří Žára, Carol O'Sullivan, "Polypostors: 2D Polygonal
Impostors for 3D Crowds", I3D 2008, 149–155, DOI 10.1145/1342250.1342273.** [paper] (search record;
the author's page returned 404)

Characters are converted into a few 2D textured polygons animated by moving vertices and UVs; similar
speed and fidelity to image impostors with up to 30 times less texture memory; the animation must be
known when the impostor is built.
*Bearing:* a middle option between baked meshes and image cards; at today's triangle budgets a
300-triangle VAT mesh does the same job with fewer constraints, so cards are kept for the last band.

**Epic Games, `AnimToTexture` (`UAnimToTextureBPLibrary`) and the Animation Sharing plugin.** [docs]
(both read)
<https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/AnimToTextureEditor/UAnimToTextureBPLibrary> ·
<https://dev.epicgames.com/documentation/unreal-engine/animation-sharing-plugin-in-unreal-engine>

AnimToTexture bakes a series of animation frames into a vertex animation texture holding either vertex
positions and normals or bone positions and rotations, normalised by a bounding box, with a current
and previous frame (for motion blur) in its playback data. Animation Sharing evaluates one pose per
state bucket and copies it to every follower, so going from 100 to 1 000 characters adds little cost,
with a cap on concurrent blends (extra blends snap).
*Bearing:* bone mode is the one for Forge: one bone texture per clip serves every body and outfit on
the crowd skeleton; the previous frame gives the motion vectors TAA needs.

**Epic Games, "Getting Started with MetaHuman Crowds" (5.8); Dean Takahashi, GamesBeat, 17 June 2026;
Amber Rutherford, 80.lv, 18 June 2026.** [docs] [web] (all read; Epic's own 5.8 news page redirected to
a login)
<https://dev.epicgames.com/documentation/metahuman/getting-started-with-metahuman-crowds-in-unreal-engine> ·
<https://gamesbeat.com/epic-games-launches-metahuman-5-8-to-create-real-time-game-character-crowds/> ·
<https://80.lv/articles/populate-your-ue5-8-worlds-with-metahuman-crowds>

Crowds (experimental) are built from MetaHuman *Collections* (modular heads, bodies, hair and clothing),
one *Instance* per distinct appearance, a Mass entity config and a spawner; people swap between
full-quality actors and lighter instanced skinned meshes by camera distance, with Animation Blueprints
near and simple sequences far, Nanite where available; the claim is hundreds of characters on mobile
and thousands on high-end platforms; new scalable normal maps cut face texture memory by about ten
times (GamesBeat's paraphrase).
*Bearing:* the closest shipped design to Forge's needs, published four months ago, and still
experimental: a reference, not a solved problem.

**The ladder, sized for Forge** *(estimate; 2 560 × 1 440, 60° vertical field of view, so a 1.75 m
person is 2 182/d pixels tall at d metres, a 0.23 m head 287/d, a 5 cm mouth 62/d)*:

| Band | Distance | Person / head on screen | People in view (street; busy square) | Body and face | Animation | Hair | Sun shadow (D-029) |
|---|---|---|---|---|---|---|---|
| **Hero** | ≤ 3 m | ≥ 730 px / ≥ 96 px | 1–4 | DNA head LOD 0–1, body LOD 0–1, full garments, 12 influences, blend shapes at LOD 0 | full stack at 60 Hz: clips, IK, look-at, RigLogic, later visemes and cloth | cards LOD 0 (strands later) | BLAS refit each frame |
| **Near** | 3–15 m | 145–730 px / 19–96 px | 5–30 | head LOD 2–3, body LOD 1–2 | full stack at a budgeted rate; RigLogic at LOD | cards LOD 1–2 | refit every frame or two |
| **T1 edge** | 15–150 m | 15–145 px / 2–19 px | 30–150 (the T1 cap) | head LOD 4–6 (84–41 joints), body LOD 3, merged garments | skeletal at 10–30 Hz by significance; feet IK to ~40 m; face still except blinks to ~20 m | cards LOD 3–4 | the nearest pre-built pose (below), or refit every N frames |
| **Crowd** | 150–400 m | 5–15 px | 300–2 000 | one merged crowd mesh per outfit archetype, 300–1 500 triangles, tints per person | bone-mode VAT clips with phase, no IK | in the mesh | pre-built poses |
| **Far** | 400 m–1.5 km | 1.5–5 px | 1 000–15 000 | 8-direction impostor card (or a 100-triangle mesh) with per-person top and bottom colours | 4–8 frame cycles | none | none, or a contact blob |
| — | > 1.5 km | < 1.5 px | — | nothing (T0's radius) | — | — | — |

- **Pre-built shadow poses** (a design for Forge, not found in a source): the crowd's shadow casters
  are a small set of BLASes, one per (clip, key frame, silhouette class) of a ~500-triangle proxy, built
  once; each far person's TLAS instance points at the BLAS of its current clip frame. No per-frame BLAS
  work, and a TLAS rebuild over a few thousand instances. Twelve clips × 16 frames × 4 silhouettes (slim,
  broad, long coat, dress) is 768 BLASes, about 40 MB at ~50 KB each *(estimate)*.
- **Per-frame cost** *(estimate)*: skinning ~2.1 million vertices (4 hero × 100 000, 25 near × 30 000,
  120 T1-edge × 8 000) at ~100 bytes each is ~210 MB of traffic, ~0.25 ms of the RTX 5070 Ti's
  896 GB/s, so 0.3–0.6 ms with the previous positions; crowd and far bands are 2–3 million triangles of
  instanced mesh-shader work; CPU animation ~150 × 20–50 µs = 3–7.5 ms of work spread over the workers
  (the sibling's figure for T1). RigLogic per face is unpublished and must be measured.
- **Memory** *(estimate)*: unique geometry only for the ~30 hero and near people whose face and body
  are blended per person (~0.6 MB each, ~20 MB); shared skin texture sets, garment masks and tileable
  fabrics 0.5–1 GB; bone textures for 40 clips on a 60-bone crowd skeleton at 60 frames are ~2.3 MB.
- **No pops:** every band boundary is a cross-fade or a dithered swap judged with ꟻLIP in motion; the
  skeletal-to-VAT swap keeps the same clip and phase (Epic's synchronised playback), and an
  outfit's crowd mesh is baked from the same garments, so silhouettes match.

### 1.6 What shipped games and demos do for thousands of visible people

**Garth Franklin, "More Details On 'The Matrix Awakens'", Dark Horizons, 14 December 2021 (from
Variety).** [web] (read)
<https://www.darkhorizons.com/?p=151475>

The demo city reportedly holds 35 000 simulated MetaHuman pedestrians, 17 000 traffic vehicles and
45 073 parked cars on 15.79 km² with 7 000 buildings; players can change crowd and traffic levels. A
community answer on Epic's forum (search record) says close people are fully rigged and far ones are
vertex-animated static meshes moved by Mass; the extracted crowd pack's Fab listing (403, search
record) describes head and body meshes adapted from MetaHumans with tops, bottoms and shoes fitted to
six body types.
*Bearing:* six body types and a modular wardrobe were enough for a demo seen from a car; a walking
first-person player sees more.

**James Vango-Brown (IO Interactive), "Create a Scalable and Destructible World in HITMAN 2", GDC 2019
(Intel-hosted slides).** [talk] (search record; the PDF exceeded WebFetch's limit)
<https://www.intel.com/content/dam/develop/external/us/en/documents/06-create-a-scalable-and-destructible-world-in-hitman2-807276.pdf>

Hitman: Absolution had 1 200 crowd characters a zone and at most 500 on screen; HITMAN's second season
1 700 a level by default and 700 on screen; a Hitman 2 level may hold 300 NPCs besides the crowd; crowd
simulation scales with three CPU tiers. Fauerby's 2012 talk on the same crowds is in `animation.md` §5.
*Bearing:* 500–700 people on screen at once is the shipped ceiling of a third-person stealth game; a
first-person street rarely shows more, and a festival square shows thousands mostly as heads and
shoulders (§4.7).

**Tommy Thompson, "How Watch Dogs: Legion's 'Play as Anyone' Simulation Works", Game Developer, 9
December 2020; Seasoned Gaming on Clint Hocking's EDGE interview, 27 June 2019.** [web] (both read)
<https://www.gamedeveloper.com/design/how-watch-dogs-legion-s-play-as-anyone-simulation-works> ·
<https://seasonedgaming.com/2019/06/27/watch-dogs-legion-how-ubisoft-is-ensuring-npc-variety/>

Each character gets tags for gender, fashion, occupation, income and personality; three tiers
(spawned and animated on screen, "uprezzed" into the deeper simulation with their connections, and
storage); a test generated over 100 lawyers who had to read as lawyers yet look distinct; each line is
recorded about 20 times and formant modulation makes one actor sound like several; cutscenes adapt to
operatives of different sizes and mobility without re-capturing. Photogrammetry captured a few dozen
faces, combined into thousands of distinct heads. Census itself is in the population note.
*Bearing:* the variety recipe at city scale (a few dozen scanned faces blended, fashion as a tag
derived from the person, voices multiplied by processing) and the test to copy (generate a hundred of
one occupation and look at them side by side).

**CAVE Academy, "CAVE supporting Clear Angle Studios and Cloud Imperium Games", 12 October 2024.** [web]
(read)
<https://caveacademy.com/cave-supporting-clear-angle-studios-and-cloud-imperium-games/>

For Star Citizen's DNA pool expansion and customisation tools, the team made 60 neutral heads with 70
FACS shapes each (4 200 assets), from scan capture to texturing, in 2021. Star Citizen's
Alpha 3.5 notes (search record, DSOGaming) describe choosing a head from the pool and blending facial
features.
*Bearing:* a pool of about 60 scanned archetypes with a shared expression set is what a AAA studio
built for blending faces; the same order of magnitude as Watch Dogs' few dozen.

**inZOI ModKit, "Facial hair: overview".** [docs] (read)
<https://mod-docs.playinzoi.com/docs/modkit-docs/modkit/how-to/caz/facialhair/01overview>

inZOI's face modding starts from Epic's MetaHuman 5.6 face modding guide; inZOI keeps LODs 0–4 of the
eight MetaHuman face LODs, and a cleanup step drops the rest.
*Bearing:* a shipped life simulation of a whole city built on MetaHuman-derived faces, keeping only
five face LODs; its far crowds therefore use something else.

**Also in the sibling notes:** Assassin's Creed Unity's 40 full AIs, 120 high-resolution models and
10 000 crowd NPCs (`game-ai-ecosystems.md` §3); GTA V's ambient density (`smart-systems.md` §3);
Cyberpunk 2077's communities and Vuntra City's ~100 spawned of four million (population note §4).
Nothing official describes GTA 6's crowds (Checked and left out).

---

## 2. The authoring pipeline and how much variety a million people need

### 2.1 MetaHuman out of Unreal, for the local sandbox

The export path (assets note §2.1) is DCC Export: head and body DNA, textures, JSON; OpenRigLogic's
README adds that grooms and clothing are not in that package. In practice a sandbox needs three
exports per archetype: the DNA pair and textures (DCC Export), the garments and hair cards as skeletal
meshes (Geometry Export or FBX from the project), and the material parameters (Materials Export). Two
newer facts matter:

**MetaHuman 5.6 and 5.8 features, from Epic's announcements as reported.** [web] (GamesBeat read; the
5.6 news and release notes seen as search records)
- 5.6 (June 2025) added a parametric body system (body shapes over a continuous range, built from
  scanned data) and parametric clothing that adapts outfits across those bodies; MetaHuman Animator
  can make real-time animation from most cameras or from audio, with offline processing kept.
- 5.8 (June 2026): Mesh to MetaHuman takes bodies as well as heads, of any topology; MetaHuman
  Animator captures face and body from one webcam, offline, through Meshcapade's markerless capture
  (experimental); Animator runs on Linux and macOS.
*Bearing:* archetypes for a sandbox can be made in Creator (faces, bodies, skin), and body motion for
small actions (sitting down, a toast, a wave) can be captured with a webcam. Both outputs are class 2,
and the EULA forbids using MetaHuman characters or animation curves to train AI (assets note §2.1).

**The cook, for Forge:** DNA → `forge-geom` reads the Geometry layer once per archetype per LOD and
cooks skinned cluster meshes (twelve or eight influences, sparse blend shapes at LOD 0); the Behavior
layer is stored as the rig's data; garments and cards become skinned meshes on the body skeleton with
per-garment body-shape morphs (§3.1); textures go through the existing pipeline with masks for tint.
Everything is keyed in the derived-data cache (D-053), and nothing class 2 enters the public
repository (D-048).

### 2.2 Class 1 placeholders and public demos

MPFB, Anny, MHR and Rocketbox are covered in the assets note (§2.2–§2.6, §3) and not repeated. Two
additions:
- **MakeHuman's MakeClothes** fits CC0 and CC-BY garments by binding each garment vertex to three body
  vertices (§3.1). Anny shares MakeHuman's topology, so MPFB's garments follow an Anny body through
  the same binding.
- **Public demos and screenshots** of the engine use these; MetaHuman appears only in the local
  sandbox and its private captures.

### 2.3 How many faces and bodies a million people need, and how variety is made

- **A seed, not an asset, per person.** Appearance is derived from the person's seed (population note
  §1.2's lazy detail): body shape (8–16 parameters), face as weights over a few archetypes, skin tone
  and texture set, eye colour, hair style and colour, facial hair, a wardrobe per occasion with
  colours, voice, gait style. About 70 bytes if stored *(estimate)*; stored only when it changes (a
  new haircut, a bought jacket).
- **Faces: blends of archetypes.** RigLogic's design document says its fixed joint-group layout makes
  matrix merging in GeneSplicer (MetaHuman's face mixer) work without densifying, and the DNA
  Descriptor carries compatibility parameters for mixing DNA files. The open library does
  not include the mixer. Forge's version: blend the neutral geometry and joint positions of up to four
  archetypes per person at the person's LOD (the behaviour matrices shared), at promotion time, for
  the ~30 hero and near people only. Farther faces use their archetype's mesh with the person's tone
  and textures.
- **How many archetypes.** Watch Dogs: Legion used a few dozen, Star Citizen 60. With *K* archetypes and
  ~150 people near a player, each archetype's family appears 150/*K* times: 19 times at *K* = 8 (the
  "twins" a player notices), about twice at *K* = 64 *(estimate)*. Target 48–64 face archetypes for
  the sandbox, spread over ages, sexes and ancestries, and 30 or more hair styles per sex and age band.
- **Exact clones.** If people draw uniformly from *N* distinct appearances, the chance that two of
  *k* = 150 near people match exactly is 1 − exp(−k²/2N): 68 % at *N* = 10⁴, 11 % at 10⁵, 1.1 % at
  10⁶ *(estimate)*. Twelve tops × ten bottoms × eight shoes × six outer layers with sixteen colours
  for three of them is 5 760 × 4 096 ≈ 2.4 × 10⁷ outfits before faces and hair *(estimate)*, so exact
  clones are not the risk; repeated *salient* pieces are (the same red coat three times in one view).
  A view-level check that spreads conspicuous garments is cheap: at promotion, swap a garment if the
  same conspicuous one is already within the player's view.
- **Bodies.** A parametric body space (MetaHuman's parametric body for the sandbox, Anny's phenotype
  parameters for class 1) sampled from height and build distributions per age and sex, with the
  crowd band reduced to a few silhouette classes (the City Sample's six body types).
- **Texture variation.** A shared library of skin texture sets by age band and type with per-person tone,
  redness and freckle parameters (MetaHuman's skin controls offer a texture "flipbook" that changes
  wrinkle density with apparent age, plus freckles and accents: search record of Epic's page, which
  rendered empty when fetched); tint masks on garments (§3.2).
- **Ageing.** Age moves four things together: the body and face shape (Anny covers infant to elder;
  MetaHuman by archetype and parametric body), the skin texture set, hair colour (melanin and grey
  fraction as parameters of the hair shader) and motion (speed, stride and posture by age band; Watch
  Dogs: Legion's cutscenes adapt to operatives of different sizes and mobility).

---

## 3. Clothing at scale

### 3.1 Garments that fit any body, in layers

**MakeHuman community, "Introduction to MakeClothes".** [docs] (read; undated)
<https://static.makehumancommunity.org/assets/creatingassets/makeclothes/introduction.html>

Each garment vertex takes its position from three vertices of the base mesh (or a helper), so
garments follow every body target; body vertices under opaque clothes can be deleted; a z-depth orders
layers, about 31 for underwear, 50 for normal clothes and 63 for coats.
*Bearing:* the open, long-standing method for fitting: bind each garment vertex to a body triangle
offline (barycentric weights plus an offset along the normal), then transfer the body's shape
parameters to the garment as its own morph targets. At run time the garment is just a skinned mesh
with the same shape weights as its wearer, which Forge's skin pass already evaluates (#169). It needs no
run-time cage solve (Roblox's patent) and no shared image-space deformation maps (EA's patent).

**REDmodding Wiki, "Garment Support: How does it work?" (Cyberpunk 2077).** [docs] (read)
<https://wiki.redmodding.org/cyberpunk-2077-modding/for-mod-creators-theory/3d-modelling/garment-support-how-does-it-work>

The engine morphs garments so they do not clip (pants tucked into boots), through per-mesh parameters
that appear as shape keys; each item gets a garment score from its slot prefix (shoes 50, legs 60,
torso inner 70, torso outer 120, head outer 110…) plus a tag (tight −1 000, large +1 000, extra-large
+2 000); items with a higher score squash those below. It applies only to items equipped through the
player's transaction system, so as of October 2024 it did not apply to NPCs.
*Bearing:* layering as data: an integer order per garment and a "squash under" morph per garment
that is weighted on when a higher layer covers that region. A shipped AAA game kept it for the player,
which says it is not free; for crowds, precompute the squashed garments per outfit when the outfit is
assembled.

**Sri Nair (Electronic Arts), "Innovations in The Sims 4 Character Creator", GDC 2015.** [talk] (GDC
Vault abstract read)
<https://gdcvault.com/play/1022085/Innovations-in-The-Sims-4>

A data-driven system of selectors and modifiers (morph targets, bone-driven deformations and
"deformation maps"), asset pairing rules for parts that occupy the same area (which one shows over the
other), direct push-and-pull sculpting, and "smart randomization" that combines assets matching
attributes into consistent looks.
*Bearing:* "smart randomization" by attributes is the wardrobe generator of §3.3; the deformation maps
themselves are under EA's patent to 2035 (Patents).

**Epic Games, "Creating Parametric Clothing for MetaHuman" and `ChaosSizedOutfitSource`.** [docs] (both
read)
<https://dev.epicgames.com/documentation/metahuman/creating-parametric-clothing-for-metahuman> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/ChaosSizedOutfitSource>

Parametric outfits (5.6) adapt to a wider range of body sizes; each size is the same garment authored
on a source body, and resizing uses interpolation points spread evenly over the body, more of them
costing generation time and disk; where more points do not give acceptable results, the reference
recommends adding another authored size. A forum thread (search record)
reports that a resized outfit exported from Creator loses its cloth simulation.
*Bearing:* even Epic authors a garment at a few sizes and interpolates between them. For Forge: author
each garment on two to four body sizes where the shape changes a lot (very tall, very broad, child),
and bind the rest.

**GarmentCode** (MIT, ETH Zürich; assets note §2.7, procedural-content §3.7) remains the class 1 seed
for parametric patterns, simulated offline into garments for each body size.

### 3.2 Material and colour variation

- **Tint masks:** each garment carries a mask of up to three regions (main, secondary, trim) and a
  fabric class (cotton, denim, wool, leather, synthetic, knit); the person's seed picks a palette
  entry per region and a pattern (plain, stripe, check, print) from a small library of tileable fabric
  detail maps. Fabric classes map to Unreal-style cloth shading (a fuzz sheen layer for cloth, per
  Epic's shading models page) in Forge's material classes (D-026).
- **Palettes are data per culture and season**, weighted by personality (openness raises saturation
  and pattern; conscientiousness lowers wear), and by occasion (§3.3).
- **Wear and fit:** a per-person wrinkle-normal blend and a slight fade parameter; no torn or dirty
  defaults in a friendly setting, which is a palette choice, not code.
- **Salience first** (McDonnell 2009): spend palette variety on tops and headwear, keep bottoms and
  shoes in narrower palettes.

### 3.3 Wardrobes by personality, occupation, season and occasion

- **The Sims 4's outfit slots** (search records of community guides): everyday, formal, athletic,
  sleepwear, party, bathing, career, situation, swimwear, hot weather and cold weather (the last two
  from the Seasons pack), with automatic changes for weather, exercise, swimming and work. Vuntra City
  added an outfit system in which people change at the spa (population note §4).
- **For Forge, a person has one outfit per occasion slot**, generated lazily from the seed: *work*
  (from occupation: a uniform list for staff jobs, a dress code for offices), *everyday*, *night out*,
  *sport*, *swim/spa* (a robe and swimwear), *sleep*, and a *weather layer* (a coat or rain jacket for
  cold or rain, none in heat). Occupation fixes the work outfit's garments; personality and age weight
  the everyday and night-out choices; season picks layers; the activity in the day plan picks the slot.
- **Changing clothes is an activity** at home, at work (a locker) or at the spa, so the person seen
  arriving in everyday clothes is seen leaving in them too. A T3 person's current slot is one byte.
- **Tone rules as data:** the slot generator has no sexualised defaults for anyone; swimwear appears
  only at pools, beaches and spas; night-out outfits draw from the same modest-to-dressy range for all
  sexes. These are table entries the owner can read.

### 3.4 Cloth simulation for heroes, skinned or baked cloth for crowds

**Epic Games, "How to Use the Machine Learning Deformer".** [docs] (read)
<https://dev.epicgames.com/documentation/en-us/unreal-engine/how-to-use-the-machine-learning-deformer-in-unreal-engine>

The ML Deformer learns from offline simulation to approximate high-quality deformation at run time: a
Neural Morph Model for flesh, an experimental Nearest Neighbour Model meant for cloth; the output is
morph targets on top of linear skinning, mostly on the GPU; the
training Alembic can reach 50–100 GB.
*Bearing:* the middle tier between simulation and rigid skinning has the same output shape as Forge's
morph targets (#169), so a learned cloth model could plug in later; it is not a first step.

**The plan for Forge:**
- **Hero (≤ 3 m), loose garments only:** coats, skirts, scarves on Jolt soft bodies with skinning
  constraints (`animation.md` §6), at most a few garments in the scene.
- **Near and T1 edge:** skinned garments with authored secondary bones (a coat's tails, a skirt's
  panels) driven by springs, the cheap procedural layer of D-012.
- **Crowd and far:** garments merged into the crowd mesh and moved by the VAT clip; loose garments
  baked with their motion in the clip.

---

## 4. Animation

### 4.1 Locomotion

Motion matching, inertialization and the Fortnite figure (every one of a match's 100 characters plus NPCs) are in
`animation.md` §2; D-012 waits for capture data before building it, and Phase 7 puts it on the player
first. For people, two additions:
- **Style by age and personality:** one database per broad style (young, adult, elderly; brisk,
  relaxed), or one database with a style channel in the feature vector; Guy et al.'s personality-to-
  motion mapping (population note §3.2) gives the speed and personal space.
- **Patents:** EA holds motion-matching patents on *local bone phases* and *periodic autoencoders*;
  plain motion matching (Clavet 2016; Holden's MIT reference) does not use either (Patents).
- **Until capture exists:** the blend space and clips Forge has, with Quaternius or Rocketbox clips
  (assets note §2.6) for the public engine and MetaHuman Animator or a bought library for the sandbox.

### 4.2 Furniture and smart objects: sitting, eating, drinking, working, dancing

**Epic Games, Contextual Animation plugin, `FContextualAnimSet`.** [docs] (read; the plugin is
experimental)
<https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/ContextualAnimation/FContextualAnimSet>

An interaction set holds one animation track per role, a map of named warp targets to transforms, and
a random weight that breaks ties between variants; actors are bound to roles
when the scene starts.
*Bearing:* the data shape for every interaction: a smart object (`game-ai-ecosystems.md` §1, with its
reservation) owns *slots* (seat 1–4 of a table, the counter's service side, a spot on the dance floor);
each slot carries a scene of role tracks (enter, loop variants, exit) and warp points; motion warping
(`animation.md` §5) lands the root on the slot whatever the approach.

The interaction catalogue for a city's people, each a scene of enter–loop–exit clips plus IK targets
*(proposed)*:
- **Sit and stand** at chairs, benches, bar stools, sofas, a tram seat; hands to the table's edge by IK.
- **Eat and drink** as additive upper-body loops on the seated base: cup and glass held in a hand
  socket, raised on a timer jittered per person; cutlery as two hand props; a toast as a synchronised
  group event (§4.3).
- **Work at a counter** (barista, bartender, shop clerk): a staff-side slot with a short loop library
  (make a drink, hand over, wipe, wait), ordered by the customer's interaction.
- **Dance:** a floor area that hands out spots on a loose grid; dance clips authored at a known tempo
  and played at the music's tempo over the clip's (rate = BPM_music / BPM_clip), phase-locked to the
  bar; a per-person style and energy from personality. No class 1 dance capture was found; AIST's dance
  database, the source of AIST++, is for academic research only (Checked and left out).
- **Garden and dog** (§4.7).

### 4.3 Walking together, conversation circles and a table for four

**Mehdi Moussaïd, Niriaska Perozo, Simon Garnier, Dirk Helbing, Guy Theraulaz, "The Walking Behaviour of
Pedestrian Social Groups and Its Impact on Crowd Dynamics", *PLoS ONE* 5(4): e10047, 2010, DOI
10.1371/journal.pone.0010047.** [paper] (read)
<https://journals.plos.org/plosone/article?id=10.1371/journal.pone.0010047>

Groups make up 55 % of pedestrians at low density and 70 % at moderate density; groups of two to four
walk abreast at low density; at higher density threes form a forward V (about 108°) with the middle
person behind, fours a U; speed falls linearly with group size (about 1.26 − 0.04 n m/s at low
density); a model with a term for members wanting to see each other reproduces the V, which slows the
crowd by about 17 % against the inverse V.
*Bearing:* the acceptance test and the controller for "walking together": a group is one navigation
agent (its centre) with member slots abreast, bending to a V or U as local density rises and to single
file in a narrow passage; members steer to their slots with their own avoidance. Group speed is the
slowest member's.

**Adam Kendon's F-formation (1990), as described in later work; Francesco Setti, Chris Russell, Chiara
Bassetti, Marco Cristani, "F-formation Detection: Individuating Free-standing Conversational Groups in
Images", arXiv:1409.2702 (2014); Claudio Pedica, "Spontaneous Avatar Behavior for Human Territoriality",
IVA 2009, LNCS 5773, 344–357, DOI 10.1007/978-3-642-04380-2_38.** [book] [paper] (Kendon's definitions
from search records of surveys citing Kendon; Setti et al.'s abstract and Pedica's record read)
<https://arxiv.org/abs/1409.2702> ·
<https://iris.landsbokasafn.is/en/publications/spontaneous-avatar-behavior-for-human-territoriality-2/>

Kendon: an F-formation exists when two or more people keep a spatial and orientational arrangement in
which each has equal, direct and exclusive access to the space between them; an inner o-space, a ring of participants (p-space), an outer r-space; vis-à-vis, L-shaped,
side-by-side and circular arrangements. Setti et al. detect such groups from position and orientation
alone; Pedica's avatars keep a conversation's cohesion, equality and spacing by blended reactive
behaviours, which read as signs of life.
*Bearing:* a standing conversation is a small smart object made on the fly: a centre, a radius from the
member count (about 0.6–0.9 m for two to five people *(estimate)*), member slots on the ring facing
the centre; a newcomer opens a gap; members shift weight and re-face when someone leaves. The same
structure serves a table (fixed slots) and a bar's standing group (free ring).

**A night out in animation terms** *(proposed)*: the group's appointment (population note §3.3) makes
one *group* entity; it walks as §4.3's formation; at the venue the group reserves a table smart object
with N seats as one reservation (or a standing ring by the bar); seated members play conversation
loops with gaze to the speaker (§5.1) and additive drinks; a dance-floor move takes the group's spots
together; leaving is a group decision (the first to tire proposes, agreeable members follow), and the
group splits at the tram stop or the door into individual trips.

### 4.4 Reactions and gestures

- **Reactions** are additive upper-body clips and look-ats triggered by events (a fall nearby, music,
  fireworks, a friend's arrival), chosen by personality (the population note's neuroticism and
  agreeableness), with the full-body ones (a step back, a flinch) through the inertialized blend.
- **Gestures during speech** come with the line (§5.1): The Witcher 3's generator placed idles and
  gestures from the voice-over; a line's tags name a gesture class (point, shrug, open palms, count
  on fingers), and the speaker's personality scales amplitude.
- **Secondary life:** blink timers, breathing, weight shifts, phone glances, all additive and all off
  past the near band.

### 4.5 First-person body awareness

**Epic Games, "First Person Rendering".** [docs] (read; 5.8 page, self-shadow controls since 5.6)
<https://dev.epicgames.com/documentation/unreal-engine/first-person-rendering>

Selected primitives render with their own field of view, different from the scene's, and are scaled
toward the camera so they draw over the world; a separate World Space Representation mesh, hidden
from the owning player's camera, casts the player's shadow and
appears in reflections; a per-vertex alpha blends between world and first-person space, so legs can
stay in world space with the feet on the ground.

**Nathan Grayson, "Star Citizen's First-Person View Is Based On Birds", Kotaku, 23 September 2016.**
[web] (read)
<https://kotaku.com/star-citizens-first-person-view-is-based-on-birds-1787021141>

Ivo Herzeg (lead animation engineer) on putting the camera in the head of the full-body character: at
first the view bobbed like a bobblehead's; counter-rotating the eyes toward a distant focus point
removed about 80 % of the bouncing; the rest needed the head itself kept stable, as birds
do.
*Bearing:* Forge's first person is the full body with the camera at the eyes, stabilised (eye then
head), with the arms and held props drawn in a first-person field of view, the legs in world space,
and the shadow from the world-space body. Third person later is the same body with another camera.

### 4.6 An animation LOD and budget

The Animation Budget Allocator and Mass's LOD are in `smart-systems.md` §3; MetaHuman's table (§1.1)
shows Epic turning off physics after LOD 2 and correctives after LOD 1. For Forge *(proposed)*:
- **Significance per person** from distance, screen size, whether the player looks at them, whether
  they are talking to the player, and whether they are a friend; read by the face rig, the clip rate,
  IK, secondary motion and the network (§7).
- **Rates:** hero and near at the frame rate; T1 edge at 30, 15 or 10 Hz by significance with
  interpolation; RigLogic only where the face is 20 px or more; IK feet to ~40 m; secondary motion to
  ~15 m.
- **A budget in milliseconds** on the CPU animation jobs and on the skin pass, with Epic's
  anti-flicker throttles (a person may change rate at most once a second).

### 4.7 Crowds at events and people in green places

**G. Keith Still, "Crowd density".** [web] (read)
<https://www.gkstill.com/Support/crowd-density/CrowdDensity-1.html>

Event safety is typically assessed at 2 people per m² (UK event guidance); a moving queue at 4; the
Green Guide's standing areas at 4.7; 5 per m² as the upper limit for standing and viewing.
*Bearing:* a festival of 10 000 on a square needs at least 5 000 m² at 2 per m²; at 2–4 per m² people
mostly see heads and shoulders, so the event band can draw rows of upper bodies (heads, hair and tops
are the salient parts anyway) and spend skeletons on the ~150 nearest. Event behaviour is data:
attraction points (stage, stalls) with a density field, idle crowd loops (sway, clap, cheer) phase-locked
to the music for the far band, and egress as a timed T3 event so the crowd leaves on trams and on foot.

- **Markets:** stalls are smart objects with a vendor slot and a customer queue; browsing is a slow walk
  with stops at stall fronts; the far band plays "browse" loops.
- **Community gardens:** long activities (dig, water, weed, harvest, chat over the fence) on garden-bed
  smart objects with tool props, booked by retirees and families in their day plans.
- **Walking a dog:** a person and a dog as one T1 group; the dog is a quadruped agent on a leash
  constraint (a rope of fixed maximum length, D-012's procedural layer for the arm), with sniff stops
  that pull the person to a halt and greetings between dogs that stop both owners for a short chat if
  they know each other. In the far band the pair is one instance with a paired clip.

---

## 5. Conversation, dialogue and the in-world device

### 5.1 Staging scripted conversations

**Piotr Tomsiński (CD Projekt RED), "Behind the Scenes of Cinematic Dialogues in 'The Witcher 3: Wild
Hunt'", GDC 2016; Game Anim's report of the talk, 23 March 2016.** [talk] [web] (Vault page and the
report read)
<https://gdcvault.com/play/1022988/Behind-the-Scenes-of-Cinematic> · <https://www.gameanim.com/?p=10962>

Over 35 hours of dialogue, 2.5 hours of cutscenes and 1 463 systemic dialogues; an auto-generator
builds a first pass *from the voice-over audio*: camera cuts by rules (the 180-degree rule,
establishing shots), idles and gestures placed; re-running gives a different result and shots swap from
a menu; 35 idle variations per skeleton type grouped by emotional state or status, sitting variants;
about 2 400 dialogue animations shared across characters through additive conversion, override
blending and bone masks; pose-based look-ats for special cases like sitting.
*Bearing:* scripted conversations at a city's scale are systemic: lines with ids and tags, a generator
that stages them on whatever slots the speakers occupy (a table, a ring, a counter), from a library of
idles and gestures by emotion. In first person there are no cuts: staging means positions, gaze
(speaker, listener, glances to the player) and gestures.

### 5.2 Barks and greetings by relationship

Ruskin's fact-matched rule database (`game-ai-ecosystems.md` §1) is the mechanism; what this topic adds
is the facts:
- `relation(speaker, target)`: stranger, neighbour, regular, colleague, friend, close friend, family;
- `familiarity(speaker, player)` and `last_met_days` (§6);
- the place and role (staff at work, a resident at home, a guest), the hour, the occasion;
- the speaker's mood and personality.
Rules from specific to general: the barista's "the usual?" (regular, at work, morning), a neighbour's
nod with the player's name (neighbour, familiarity ≥ 3), a friend's "you made it!" at a meeting
(appointment, arrived), then generic courtesy.

**Alessio Palumbo, "Rockstar Explains How The New NPC Interaction System in RDR2… Came To Be",
Wccftech, 25 September 2018 (quoting Official Xbox Magazine).** [web] (read)
<https://wccftech.com/rockstar-explains-npc-interaction-rdr2/amp/>

Imran Sarwar (Director of Design) says giving the gang better memories (they know when Arthur is in
camp and when he has caused trouble elsewhere) led to an interaction system for the whole
world: shopkeepers, lawmen and riders can be engaged several ways without a weapon, depending on the
person and situation.
*Bearing:* greetings that remember are what make a crowd feel like neighbours; they cost a fact lookup.

### 5.3 A data format for scripted dialogue

**Yarn Spinner (`YarnSpinnerTool/YarnSpinner`, MIT) and Yarn Spinner for Rust
(`YarnSpinnerTool/YarnSpinner-Rust`, Apache-2.0); inkle's ink (`inkle/ink`, MIT) and blade-ink-rs
(`bladecoder/blade-ink-rs`, Apache-2.0); Yarn Spinner's documentation on tags.** [repo] [docs]
(repositories read through `gh api`; the documentation page read)
<https://github.com/YarnSpinnerTool/YarnSpinner-Rust> · <https://github.com/bladecoder/blade-ink-rs> ·
<https://docs.yarnspinner.dev/write-yarn-scripts/advanced-scripting/tags-metadata>

Yarn Spinner's Rust port calls itself work in progress, with first-class Bevy support and a path for
use without Bevy; in Yarn a `#line` tag gives each line or option an id unique across the game's
dialogue, missing ids are added automatically, and line tags carry metadata for the game
and translators. blade-ink-rs is a Rust runtime for compiled ink.
*Bearing:* either fits D-053's content as data. Yarn's stable line ids are what the network and
localisation need (§7), and its tags carry the staging hints of §5.1 (gesture, emotion, addressee).
A choice for the owner; Yarn is the closer fit.

**What a line carries** *(proposed)*: line id; speaker role (not a fixed character: "barista",
"friend A"); conditions as facts (§5.2); text key; tags (emotion, gesture class, addressee, interruptible);
voice asset or TTS voice id; duration. Conversations between two NPCs reuse the same lines.

### 5.4 Hooks left for language models (#225)

**NVIDIA, "NVIDIA ACE Autonomous Game Characters Debut This Month In inZOI…", 13 March 2025.** [web]
(read)
<https://www.nvidia.com/en-gb/geforce/news/nvidia-ace-naraka-bladepoint-inzoi-launch-this-month/>

inZOI's Smart Zoi option runs a 0.5-billion-parameter Mistral NeMo Minitron language model on the
device (accelerated on GeForce RTX GPUs only) to drive people's actions and thoughts; at the
end of each day they adjust their schedules from their experiences, and players steer them by free
text. Ubisoft showed a similar NEO NPC prototype at GDC 2024 (search record of its press release).
*Bearing:* the first shipped life simulation with an on-device model, and it is vendor-locked; under
D-016 a model's output may enter Forge's simulation only as a recorded input.

**Hooks to leave now** *(proposed)*:
- every line goes through one `say(speaker, line)` path, whether the line comes from a script or later
  from a model, so staging, gestures, subtitles and the network do not care where it came from;
- a *line source* trait: scripted (Yarn) now; a model later that returns text plus the same tags,
  run on the client for flavour or on a service for authoring, its output logged like a player input;
- the facts of §5.2 and the memories of §6 serialisable as a short context for a prompt;
- voices: a TTS path beside recorded lines. Kokoro-82M's weights are Apache-2.0 and its training audio
  is described as permissive or non-copyrighted only (model card read); Piper's original MIT repository is
  archived and its successor is GPL-3.0 (`gh api`).

### 5.5 Visemes later: blend shapes, the MetaHuman rig and audio-to-face

- **Control sets.** Apple's ARKit 52 blend shapes (assets note §3) and Meta's 15 visemes (sil, PP, FF,
  TH, DD, kk, CH, SS, nn, RR, aa, E, I, O, U; Meta's reference page, read, now unmaintained) are the
  two vocabularies; MPFB ships both as CC0 (assets note §2.2). MetaHuman's face takes animation curves
  on its rig controls; Epic's Live Link Face app streams a phone's capture to it (GamesBeat), and
  mapping ARKit curves onto those controls is that app's long-standing path (not re-checked here).
- **MetaHuman Animator** (Unreal, class 2) makes face animation from audio, offline or in real time
  since 5.6 (search records); good for authoring recorded lines in the sandbox.
- **NVIDIA Audio2Face-3D (`NVIDIA/Audio2Face-3D`).** [repo] (read through `gh api`) Audio to jaw,
  tongue and eye motion and skin deformation as mesh deltas, joints or blend-shape weights. The SDK
  (C++ with Python), the Maya plugin and the Unreal 5.5–5.6 plugin are MIT; the training framework
  Apache-2.0; the pre-trained face models under NVIDIA's Open Model licence; the Audio2Emotion models
  usable with Audio2Face only; the sample dataset evaluation-only. It accelerates on NVIDIA GPUs with
  a CPU fallback.
- **Rhubarb Lip Sync (`DanielSWolf/rhubarb-lip-sync`).** [repo] (read through `gh api`) MIT, 2D mouth
  shapes from recordings; the output belongs to the user.
- **Patents:** JALI's jaw-and-lip viseme units, NVIDIA's emotion-driven audio-to-face and Meta's
  pitch-driven brows (Patents). An MIT licence grants copyright, not explicitly patent rights; NVIDIA's
  own release of its SDK suggests it expects use, but the owner should decide before shipping it.
*Bearing:* visemes need nothing now except that the face rig's controls are the ARKit-compatible
curves and that every line carries an audio clip or a TTS stream id.

### 5.6 The smart pad: the people and interface side of one in-world device

**Erik Fagerholt, Magnus Lorentzon, "Beyond the HUD: User Interfaces for Increased Player Immersion in
FPS Games", master's thesis, Chalmers University of Technology with EA DICE, 2009.** [web] (record and
abstract read)
<https://odr.chalmers.se/handle/20.500.12380/111921>

Proposes a definition of immersion and four guidelines (know your design space, know your game,
establish player agency, strengthen the player–avatar perceptual link); its classification of interface
elements as diegetic, non-diegetic, spatial or meta (from search records of the thesis and its readers)
is the field's usual vocabulary.

Precedents (search records): Watch Dogs' phone profiler showed each passer-by's name, age,
occupation, income and a personal fact; Star Citizen's wrist-worn mobiGlas holds contacts, chat,
invitations, a scheduler and the starmap.
*Bearing (proposed):* the pad is a held prop in first person whose screen is a UI rendered to a
texture (egui, already in `forge-app`, can draw offscreen), so the player sees the street behind it
and the time passes while they use it. The people side of its apps:
- **Messages:** threads with friends and contacts; invitations are appointments proposed by a friend's
  social planner (population note §3.3) and accepted or declined by the player, which creates or
  cancels the appointment for everyone in it; friends write first (a scripted line chosen by §5.2's
  facts), reply to the player's answers and say when they are late.
- **Calendar:** the player's appointments and their friends' shared ones; reminders before departure
  time, computed from the transit estimate (the transport topic's side).
- **Book a table:** a venue's capacity by hour (population note §3) shown as free tables; a booking
  reserves a table smart object (§4.3) for a party at a time; staff greet a booked party by name.
- **Contacts:** people the player has met, with what the player knows of them (name once introduced,
  job once told, last met), never more than the player learned: the pad must not become a profiler.
- **Hail a taxi and transit times:** the transport topic; here only that the pad's request is a player
  input like any other.

---

## 6. Relationships and memory

The cheap model for a million is in the population note §3.3 (seeded friends, implicit relations by
household, workplace and building, RimWorld-style timed memories only for pairs that changed, a small
record per (person, player)). What this topic adds is how regulars, friends and neighbours behave
toward the player, and what that costs.

**Stardew Valley Wiki, "Friendship".** [docs] (read)
<https://stardewvalleywiki.com/Friendship>

250 points a heart, ten hearts for most villagers; talking once a day +20; not talking costs −2 a day
(−20 for a spouse) unless the meter is full; gifts at most two a week (loved +80, liked +45, disliked
−20, hated −40), a birthday gift ×8; quests and events add 100–500; heart events and room access unlock
at thresholds; dialogue warms and villagers send gifts by mail as hearts rise.
*Bearing:* the model for regulars: a familiarity that grows by short, repeated contact and decays
slowly, with visible thresholds (they learn your name, they know your order, they invite you).

**Watch Dogs: Legion** (§1.6's Game Developer report): recruits and their connections are "uprezzed"
together, schedules repeat until something changes, and people a player recruited can be recast into
the world later. **RimWorld's opinion, Talk of the Town's beliefs and Census's propagated memories**
are in the population note §3.2. **The Sims 4's friendship bar** runs −100 to 100 with friend at 40,
good friend 60, best friend 80 by community guides (search records).

**What Forge keeps** *(proposed)*:

| Data | For whom | Bytes | At a million |
|---|---|---|---|
| implicit relations (household, building, workplace, school) | everyone | 0 (derived from the record) | 0 |
| seeded friends (four ids) | everyone | 16 | 16 MB (population note's column) |
| pair memories (opinion delta, kind, until) | pairs whose opinion changed | ~12 each | a few MB if 1 % of people have a few |
| **player familiarity**: person, player, familiarity 0–255, affinity −128–127, last met day, flags (knows name, knows job, regular at, invited) | each (person, player) who met | ~16 | per player: 50–200 people a session, ~10 000 over months ≈ 160 KB *(estimate)* |
| player-to-person *recorded facts* (what the person told the player) | as told | a few bytes each | with the familiarity record |

- **Regulars** emerge from the day plan, not from a flag: staff work fixed shifts at one venue, and a
  player who comes at the same hour meets the same barista. Each meeting raises familiarity (Stardew's
  +20 a day, in game days); at thresholds the barista learns the name, then the usual order, then asks
  about the player's friends.
- **Neighbours** are people of the same building or street (implicit). Familiarity grows when the
  player and a neighbour are both in T1 near home and the player greets or is greeted; a neighbour's
  greeting uses the name past a threshold.
- **A friend group** is a seeded set of 3–6 people around the player (generated when the player
  arrives, with compatible personalities and ages), with high affinity from the start and its own
  social planner entry: it proposes nights out by message (§5.6), remembers who came, and lowers
  affinity slowly for declined invitations (never punitive in a friendly setting).
- **Memory of the player** is facts with a time: met at, introduced, helped with, last night out; the
  dialogue matcher reads them; they decay by kind (a greeting is forgotten in days, a rescue in months).
- **Gossip** (a memory about the player spreading to a person's friends, Census's "intrigues") is the
  feature that most resembles Warner Bros.' Nemesis claims as described (an event with one character
  changing another's parameters); ask the owner before building it (Patents).

---

## 7. Netcode for people near players

**Valve, Source SDK 2013: `baseanimating.cpp`, `BaseAnimatingOverlay.cpp`, `baseanimating.h`,
`shareddefs.h`.** [repo] (read through `gh api`; Valve's Source 1 SDK licence; the Valve Developer wiki
refused the fetch)
<https://github.com/ValveSoftware/source-sdk-2013>

Source replicates animation as parameters, never poses: the sequence index in 12 bits (4 096
sequences), the cycle in 15 bits (10 in Team Fortress 2), the playback rate in 8 bits, pose
parameters in 8 or 11 bits each, skin and body-group selections, and per overlay layer its sequence,
cycle, previous cycle, weight and order.
*Bearing:* twenty years of shipped practice behind D-012's "replicate parameters, never poses", with
bit counts to start from.

**What crosses the wire for a T1 person** *(proposed, on top of the population note's 6.5-byte
update)*: action or clip id (8 bits) and its phase (8 bits), the interaction target (smart object and
slot, ~16 bits, only when it changes), the look-at target (a person id delta or a slot, ~12 bits, when
it changes), group membership and formation slot (on change), and reliable events for lines spoken
(line id, speaker, addressee, start tick: ~6 bytes), reactions and gestures (event id + seed). Clips,
gestures, blinks and secondary motion are chosen on each client from these and the seed. **Who
animates:** every client animates the people it draws; the server holds only capsules, slots and
conversation state (population note §5.2). Shared contact (two people clinking glasses, a handshake) is
anchored on the server-owned slot transforms, so both clients agree to the centimetre without pose
exchange.

**Bandwidth** *(estimate)*: 40 moving people within 30 m at 15 Hz × 9 bytes = 5.4 KB/s; 80 moving
people at 30–150 m at 5 Hz × 9 bytes = 3.6 KB/s; seated, queued and dancing people send only on change
(dance phase is derived from the music clock) ≈ 0.3 KB/s; lines and reactions ≈ 0.5 KB/s; about
**10 KB/s**, inside D-010's 24 KB/s with the far crowd's 1 KB/s of whereabouts events (population note
§5.3). **Other players** add their aim (yaw and pitch at 10 bits each), stance and the held prop at the
snapshot rate; their first-person arms are local only.

---

## What professional engines do

| Game or engine | Near people | Far crowd | Faces and variety | Clothes | Persistent identity |
|---|---|---|---|---|---|
| Unreal 5.8 MetaHuman Crowds | actors, Animation Blueprints, full rig | instanced skinned meshes, sequences, Mass; Nanite optional | Collections of modular heads, bodies, hair, clothing | parametric outfits by size | none built in |
| Matrix Awakens / City Sample (2021–22) | rigged MetaHuman-derived | vertex-animated static meshes, Mass (forum) | crowd pack, six body types | tops, bottoms, shoes per body type | none (35 000 simulated pedestrians) |
| Hitman (2012–2018) | full AI, up to 300 per level | 1 200–1 700 crowd per level, 500–700 on screen | — | — | none |
| Assassin's Creed Unity (2014) | 40 full AIs, 120 high-res models | 10 000 crowd NPCs | — | — | none |
| Watch Dogs: Legion (2020) | uprezzed with connections | spawned and deleted near the player | dozens of scanned faces → thousands of heads; 20 voice versions per line | "fashion" tag per person | Census profiles, schedules, memories |
| Star Citizen | full-body first person, stabilised head | — | DNA pool: 60 heads × 70 FACS | — | — |
| inZOI (2025–) | MetaHuman-derived faces, LOD 0–4 | — | Create-a-Zoi | — | a city of Zois; Smart Zoi on-device model |
| The Sims 4 (2014–) | full | lot-local | selectors, modifiers, deformation maps, smart randomisation | eleven outfit slots, weather changes | households, relationships −100–100 |
| The Witcher 3 (2015) | systemic dialogues: 1 463 | — | 35 idles per skeleton type, 2 400 dialogue clips | — | authored |
| Red Dead Redemption 2 (2018) | interaction system for everyone | ambient | — | — | memories of the player (gang, world) |
| Stardew Valley (2016) | — | — | — | — | hearts: 250 points, daily talk, gifts, decay |
| Cyberpunk 2077 (2020) | — | crowd system separate from communities | — | garment scores and squash, player only | communities rerolled on load |
| Vuntra City (in development) | ~100 spawned | GPU-evaluated schedules for 4 M | procedural appearance | outfit changes (spa) | schedules, personality, friendship |

---

## Recommendation for Forge

**Verdict.** Build the human runtime around one identity with two bodies: a skeletal person for the
few hundred near a player and a baked person for the thousands beyond, swapped without a pop by clip
and phase, as Unreal 5.8 now does. Start with one MetaHuman at a table, because skin, eyes, hair and
the face rig are the hard look problems and a café table is where the owner will judge them; then a
group, then a street. Keep the public engine on class 1 assets and the sandbox on MetaHumans.

**Crates and where things live** *(proposed)*:
- **`forge-anim`:** a `face` module (a Rust port of the DNA reader's Behavior and Geometry layers and of
  RigLogic's evaluation, credited, tested against the C++ library on the owner's machine); interaction
  scenes (roles, warp points, enter–loop–exit); group formations' animation side; first-person
  stabilisation; later motion matching.
- **`forge-geom`:** DNA geometry and garments cooked into skinned cluster meshes; offline garment binding
  and body-shape morph transfer; the crowd bake (merged outfit meshes, bone textures, shadow pose
  BLASes, impostor atlases), all through the derived-data cache.
- **`forge-render`:** a skin class with separable screen-space scattering and a transmission term; an eye
  class; hair cards; the mouth occlusion; the crowd path (instanced mesh-shader drawing from bone
  textures, with previous-frame sampling for motion vectors); impostors; first-person rendering (a
  second field of view, a world-space shadow body); twelve influences for faces.
- **`forge-people` (new):** the appearance genome from the seed; archetype, garment and palette
  libraries as data; wardrobes by occasion; the representation and LOD choice per person from one
  significance; view-level variety checks.
- **`forge-social` (new):** relations, familiarity and memories; the bark and greeting matcher's facts;
  conversation state machines with line ids; the dialogue runtime adapter (Yarn); the line-source trait
  for #225.
- **`forge-sim::population`:** the tiers, day plans and appointments of the population note, which these
  crates read.

**Build order, each step with a visible, measured result:**
1. **One person at a café table** (class 1 first: an Anny or MPFB body; then a MetaHuman in the
   sandbox). DNA loaded and RigLogic evaluated in Rust; the face at LODs 0–3 with blinks, look-at and a
   few expressions; skin, eyes, teeth and hair cards; seated in a chair smart object, drinking, glancing
   at the player and greeting with one scripted line. *Measure:* frame ms by pass (skin, SSS, hair,
   RigLogic µs), the LOD swaps by ꟻLIP in a 10 s dolly from 1 m to 30 m, close-up captures judged by
   the owner beside a MetaHuman reference render.
2. **Variety:** 30 people from 8 archetypes and 40 garments with fitted morphs and tints at the
   neighbouring tables. *Measure:* garment clipping (vertices inside the body) per outfit and body,
   cook time, memory per person, a "hundred lawyers" sheet for the owner.
3. **A friend group of four on a night out:** walk together (Moussaïd's formations), take a table for
   four, talk with staged lines and gestures, toast, dance, leave and part at a stop; two clients see the
   same night. *Measure:* group cohesion (members' distance to slots), interpenetrations, µs per T1
   person, bytes per second per client, line events in sync across clients.
4. **A street of 500 drawn people:** the skeletal-to-VAT swap, the crowd band, impostors to 1.5 km, the
   shadow pose BLASes. *Measure:* frame ms at 500, 2 000 and 10 000 drawn people on the RTX 5070 Ti,
   VRAM, the swap seen by ꟻLIP (no pop), shadow mismatch at the swap.
5. **Regulars and the pad:** a barista who learns the player's name and order over game days, neighbours
   who greet, friends who invite by message, a table booked. *Measure:* bytes per (person, player),
   memories per player after a week, invitations accepted and met.
6. **An event:** a square at 2 per m² with a stage. *Measure:* frame ms with 5 000–10 000 people, the
   band split, egress time.
7. **Later:** motion matching for the player (Phase 7) and then for people; cloth simulation for hero
   garments; visemes (ARKit curves from Audio2Face or authored); strands for heroes; a language-model
   line source (#225).

**For decisions later (D-0xx 🟡, when the first step starts):**
1. RigLogic and the DNA reader ported to Rust (recommended) or wrapped through FFI with a C++ build.
2. Twelve influences in the skin pass, or pruning to eight at import.
3. Skin: separable screen-space scattering with a transmission term (recommended), Burley's profile, or
   pre-integrated only.
4. Hair: cards only until a hero needs strands; whether an NVIDIA-only LSS path is ever wanted.
5. The crowd path: bone-mode VAT plus shadow pose BLASes (recommended), or per-person refits with CLAS
   on NVIDIA.
6. The dialogue format: Yarn Spinner (recommended) or ink.
7. Patents to clear with the owner: gossip and propagated memories (Nemesis family), any audio-to-face
   runtime (JALI, NVIDIA, Meta), garment fitting by run-time cages (Roblox) or image-space deformation
   maps (EA).

---

## What to measure

- **Face:** RigLogic µs per face at each DNA LOD (one and six workers), the DNA load time and bytes, the
  joint and blend-shape counts kept per LOD; the Rust port's outputs against the C++ library's (maximum
  absolute difference per joint and weight over a scripted control sweep).
- **Skin pass:** vertices skinned per frame, ms, with eight against twelve influences; the morph cost at
  LOD 0; BLAS refit ms per person and trace cost after a sit-down (#169's rebuild rule).
- **Shading:** SSS pass ms at 1440p with 1, 4 and 30 faces on screen; hair cards' shimmer in motion
  (ꟻLIP between consecutive TAA frames on a slow orbit); the eye and mouth close-ups against a
  reference.
- **LOD swaps:** ꟻLIP of every band boundary in a dolly; the skeletal-to-VAT swap's pose error (cm) at
  the switch; time a person spends at each band in a street walk.
- **Crowd:** frame ms and VRAM at 500, 2 000, 10 000 and 20 000 drawn people; bytes of bone textures,
  merged meshes, shadow poses and impostor atlases; draw and dispatch counts.
- **Variety:** duplicate conspicuous garments within a view (target zero), archetype family counts
  within 150 m, the owner's judgement of a 100-person sheet per occupation and age band.
- **Clothes:** body-through-garment vertices per outfit across the body space (target zero at T1),
  layering failures, cook time per garment and size.
- **Animation:** µs per T1 person by band, foot sliding (cm) and hand-to-prop error at slots, group
  formation error (m) and speed against Moussaïd's line, first-person camera jitter (deg/s at the eye).
- **Dialogue:** lines per minute in a busy bar, the staging generator's time per line, the share of
  lines with a gesture, desync of line events across two clients (ticks).
- **Relationships:** bytes per (person, player), records per player after a game week, memory decay
  checks, invitations proposed, accepted and met.
- **Network:** bytes per second per client by stream (T1 snapshots, line events, interaction changes),
  at a café, a street and a festival.

---

## Patents

Found with WebSearch and Google Patents on 2026-10-10; statuses and expiry dates are Google Patents'
estimates (it calls them assumptions, not legal conclusions), and claims were read only where the page
showed them. A list of what was met, not a freedom-to-operate review.

| Patent | Holder | Jurisdiction | Status (as listed) | Expiry (estimate) | What it touches | Alternative |
|---|---|---|---|---|---|---|
| US 11,562,523 B1, "Enhanced animation generation based on motion matching using local bone phases" (priority 2 Aug 2021, granted 24 Jan 2023); continuation US 11,995,754 B2 | Electronic Arts | US | active | anticipated 2 Aug 2041 | motion matching run per body part on *local bone phases* (contacts per limb), poses blended | global-feature motion matching (Clavet 2016, Holden's MIT demo): one feature vector, no per-bone phase channels |
| US 12,403,400 B2, "Learning character motion alignment with periodic autoencoders" (priority 31 Mar 2022, granted 2 Sep 2025) | Electronic Arts | US | active | 25 Oct 2042 | motion matching over a learned *motion phase manifold* with local phase channels (DeepPhase's line) | as above; no learned phase manifold |
| US 8,411,092 B2, "2D imposters for simplifying processing of plural animation objects…" (priority 14 Jun 2010, granted 2 Apr 2013) | Nintendo (from Retro Studios) | US | active | 3 Nov 2030 | replacing two *overlapping* animated 3D objects with one shared 2D impostor | one impostor per person (or per-person VAT meshes); never merge overlapping people into one card |
| US 7,019,742 B2, "Dynamic 2D imposters of 3D graphic objects" (priority 20 Nov 2003) | Microsoft | US | **expired** (fee-related) | 29 Sep 2024 | grouped crowd impostors projected from a shared point, batched per group | free to use |
| US 11,276,227 B2, "Object rendering method … using a simulated pre-integration map" (priority 24 Dec 2019, granted 15 Mar 2022) | Tencent | US (CN priority) | active | 10 Oct 2040 | skin colour from a *function imitating* a pre-integration map (terminator, bright and dark colours), plus a backlight term | Penner and Borshukov's 2011 lookup texture, or separable screen-space scattering (§1.3) |
| US 10,839,825 B2, "System and method for animated lip synchronization" (priority 3 Mar 2017, granted 17 Nov 2020) | JALI Inc. (from the University of Toronto) | US | active | 12 May 2037 | phonemes mapped to visemes with separate jaw and lip contributions as co-articulated "viseme action units" with listed rules | authored curves (MetaHuman Animator), ARKit-curve output from an audio model, or Meta-style visemes without the jaw/lip decomposition; check claims before any run-time lip sync |
| US 12,614,329 B2, "Audio-driven facial animation with emotion support using machine learning" (priority 7 Jul 2022, granted 28 Apr 2026) | NVIDIA | US | active | 7 Jul 2043 | a network producing motion for face components (skin, eyes, tongue) from audio with emotion and style vectors | authored animation; NVIDIA's own MIT SDK and Open Model licence do not state a patent grant (ask the owner) |
| US 10,755,463 B1, "Audio-based face tracking and lip syncing…" (priority 20 Jul 2018, granted 25 Aug 2020) | Meta Platforms Technologies (from Facebook) | US | active | anticipated 20 Jul 2038 | lip animation per speech unit plus facial components (brows, lids) driven by the unit's pitch | brows and lids from tags on the line or from emotion, not from pitch |
| US 10,926,179 B2, "Nemesis characters, nemesis forts, social vendettas and followers in computer games" (priority 26 Mar 2015, granted 23 Feb 2021); related US 11,660,540, US 12,201,908 B2, US 2025/0108303 A1 | Warner Bros. Entertainment | US | active | 11 Aug 2036 | detecting a game event involving one NPC and changing another NPC's parameters; NPC hierarchies, memories of encounters (from the description; claims not read) | memories that change only the remembering person's own opinion and lines (Stardew, RimWorld); **ask the owner before gossip or propagated memories** |
| US 12,589,309 B2, "Tailoring in-game dialogue to player attributes" (priority 10 Aug 2023, granted 31 Mar 2026) | Sony Interactive Entertainment | US | active | 1 Jun 2044 | inferring a player attribute from *sensor data* (camera, microphone) while engaging NPCs and customising NPC dialogue options | affinity and familiarity from in-game actions only; no camera or microphone sensing |
| US 11,615,601 B2, "Layered clothing that conforms to an underlying body and/or clothing layer" (priority 15 Mar 2021, granted 28 Mar 2023); continuation US 12,374,067 B2; WO, EP, CN, JP, KR family | Roblox | US and others | active | anticipated 14 Jul 2041 | each garment's inner cage mapped (RBF) to the body cage or to the outer cage of the layer beneath, at fitting | MakeHuman-style offline binding of garment vertices to body triangles plus per-garment shape morphs; layering by order and squash morphs (Cyberpunk) |
| US 9,626,808 B2, "Image-based deformation of simulated characters of varied topology" (priority 1 Aug 2014, granted 18 Apr 2017) | Electronic Arts | US | active | 4 Apr 2035 | body-shape deltas stored in shared *deformation map images* applied to garments of any topology, tight and loose maps blended by vertex colour | per-garment morph targets made offline (Forge's #169 morphs) |

**Looked for and not found** (searches only): a Ubisoft patent on motion matching or learned motion
matching; an Epic or 3Lateral patent on face-rig evaluation (a Magic Leap patent on placing rig joints
from scans, US 11,308,673, came up and is not relevant here); a patent on vertex-animation textures for
crowds; a Ubisoft patent on Census-style population generation; an Activision application on styling
NPCs from player profiles appeared as a press record only. Their absence from results is not proof. The
Take-Two and Improbable patents met in the population note are not repeated.

---

## Checked and left out

- **The RigLogic whitepaper** (`rig-logic-whitepaper-v2`, PDF): larger than WebFetch's 10 MB limit; its
  points come from 80.lv's summary and the repository's design document. No source publishes RigLogic's
  cost per face.
- **Epic's MetaHuman pages behind a login** ("The tech behind MetaHuman Creator face rigs", the 5.8
  news post): redirected to Epic's login; secondary reports used instead. The MetaHuman skin controls
  page and the City Sample page rendered with no content; the City Sample crowd pack's Fab listing
  refused (403).
- **A way to export MetaHuman grooms out of Unreal:** none documented in what was found; Epic's groom
  pages cover import only.
- **GTA 6:** nothing official on its crowds, characters or clothing; enthusiast analyses of trailers were
  left out.
- **Gray Zone Warfare:** no source on its character technology was found (only that it uses Unreal 5,
  motion capture and a "full body awareness" tag on Giant Bomb, search records).
- **Cyberpunk 2077's crowd system and Assassin's Creed Unity's crowd rendering:** not researched beyond
  the sibling notes.
- **McDonnell et al. 2009:** both repository pages refused (403); search record only.
- **Polypostors:** the author's page is gone (404); the PDF was not fetched.
- **Hitman's crowd numbers:** the Intel-hosted slides exceeded the fetch limit; search record only.
- **Frostbite hair, Part 1 and "the future of hair rendering":** both returned 404 after redirects.
- **Valve Developer Community pages:** refused (403); the SDK's code was read instead.
- **The Sims wiki** (402) and StrategyWiki (403): Sims relationship and outfit facts are search records
  of community guides.
- **A class 1 dance motion library:** none found. AIST's dance database (behind AIST++) is for academic
  research only and needs written consent for commercial use (its terms page, read); AIST++'s own
  licence was not stated on its pages.
- **The Witcher 3 talk's video and slides:** members-only; the facts come from Game Anim's report.
- **Ubisoft NEO NPC:** the GDC 2024 press release (a PDF) was seen only as a search record.
- **Meta's Lipsync licence:** not stated on the viseme page, which is no longer maintained.
- **Texture memory per person and shadow-pose BLAS sizes:** estimates; no source measured them.

---

## Verification notes

Checked on 2026-10-10 with WebFetch, WebSearch and `gh` (read-only GitHub API calls) only; no browser
pane, nothing downloaded, no repository changed. "Read" means the page, file or API response was
returned and the facts come from it; "search record" means only a search engine's extract was seen.
Quotes are short and attributed; everything else is paraphrased.

- **Read:**
  - GitHub through `gh api`: `EpicGames/openriglogic` (README, `docs/design/design.md`, layer headers,
    `examples/Basic.cpp`, tree); `EpicGames/MetaHuman-DNA-Calibration` (README, `docs/dna.md`,
    LICENSE); `iryoku/separable-sss`; `NVIDIA-RTX/RTXCR` (README, License.txt); `RoboPoets/digital_human`;
    `nvpro-samples/vk_animated_clusters`; `NVIDIA/Audio2Face-3D` and the organisation's repository
    list; `DanielSWolf/rhubarb-lip-sync` (LICENSE.md); `inkle/ink`; `bladecoder/blade-ink-rs`;
    `YarnSpinnerTool/YarnSpinner` and `YarnSpinner-Rust` (README); `rhasspy/piper`,
    `OHF-Voice/piper1-gpl`, `hexgrad/kokoro`; `ValveSoftware/source-sdk-2013` (four files, LICENSE);
    `Unity-Technologies/Animation-Instancing` (not cited); a repository search for Rust DNA readers
    (none found).
  - Epic documentation: MetaHuman DNA; Editing modes; Controlling MetaHuman LODs; Platform support and
    LOD specifications; Getting started with MetaHuman Crowds; Creating parametric clothing;
    `ChaosSizedOutfitSource`; Nanite (5.8); Subsurface Profile; Shading Models; Hair Rendering and
    Simulation; First Person Rendering; ML Deformer; `UAnimToTextureBPLibrary`; Animation Sharing;
    `FContextualAnimSet`.
  - Epic forums: "Status of Skinned Meshes (and Metahuman) and Nanite Support"; "AnimBank and Instanced
    Skinned Mesh Component".
  - Press and blogs: 80.lv (10 May 2022; 18 June 2026); GamesBeat (17 June 2026); Dark Horizons (14
    December 2021); Game Developer (9 December 2020); Seasoned Gaming (27 June 2019); CAVE Academy (12
    October 2024); Kotaku (23 September 2016); Wccftech (25 September 2018); NVIDIA GeForce news (13
    March 2025); NVIDIA Technical Blog (6 February 2025); EA's Frostbite hair Part 2 (AMP copy); Game
    Anim (23 March 2016); TechRadar (headline only).
  - Other pages: inZOI ModKit facial hair overview; iryoku.com's Separable SSS page and its GDC 2013
    post; Goldsmiths' record of McDonnell et al. 2008; PLoS ONE (Moussaïd et al.); arXiv 1409.2702 and
    2108.06181 (abstracts); Landsbókasafn's record of Pedica 2009; Chalmers' record of Fagerholt and
    Lorentzon; GDC Vault pages 1022988 and 1022085; Stardew Valley Wiki "Friendship"; REDmodding Wiki
    "Garment Support"; MakeHuman "Introduction to MakeClothes"; Yarn Spinner's tags page; Meta's viseme
    reference; Hugging Face's Kokoro-82M card; G. Keith Still's crowd density page; AIST Dance DB terms
    of use; AIST++ home and download pages.
  - Google Patents: US 11,562,523 B1; US 12,403,400 B2; US 8,411,092 B2; US 7,019,742 B2; US 11,276,227
    B2; US 10,839,825 B2; US 12,614,329 B2; US 10,755,463 B1; US 10,926,179 B2; US 12,589,309 B2; US
    11,615,601 B2; US 9,626,808 B2; US 11,308,673 B2.
- **Search record only:** McDonnell et al. 2009; Kavan et al. 2008 (Polypostors); Penner's skin slides
  and chapter; the Hitman 2 GDC 2019 slides; the City Sample forum answer and crowd pack listing;
  Star Citizen's Alpha 3.5 notes; MetaHuman 5.6's parametric body and Animator audio features;
  MetaHuman skin controls; Kendon's F-formation definitions (via surveys); the Sims 4 outfit slots and
  friendship tiers; Watch Dogs' profiler; Star Citizen's mobiGlas; Ubisoft NEO NPC; the LSS Vulkan
  reference page; Gray Zone Warfare's technology; GTA 6 trailer analyses; US 11,995,754 B2, US
  12,374,067 B2, US 11,660,540 and US 12,201,908 B2 (family members seen on the parents' pages).
- **Refused or empty:** metahuman.com pages (login redirect); the RigLogic whitepaper and the Intel
  Hitman slides (over 10 MB); Fab listings (403); docs.unrealengine.com 4.27 and 5.2 pages (403); the
  City Sample page and MetaHuman skin controls page (no content); Ladislav Kavan's Polypostors page
  (404); Trinity College Dublin's and eDeposit Ireland's repositories (403); Frostbite hair Part 1 and
  "future of hair" (404); vulkk.com (403); techraptor.net (403); tweaktown.com (403); Valve Developer
  Community (403); sims.fandom.com (402); strategywiki.org (403); Pixar's library page (redirect to a
  general page); Wikipedia's "F-formation" (404); AIST++'s factsheet (404).
- **From Forge, read in the repository:** `CLAUDE.md`; `docs/research/galaxy-sky.md` (the format),
  `assets-and-licences.md`, `population-simulation.md`, `procedural-content.md` §3.7 and Patents,
  `animation.md`, `game-ai-ecosystems.md` §1 and §3, `smart-systems.md` §3, `netcode.md` (entry list);
  `docs/DECISIONS.md` D-010, D-012 (with its amendment and #165/#167 notes), D-052; `docs/ROADMAP.md`
  Phase 7; `docs/ARCHITECTURE.md`'s crate rows; issues #168, #169 (with comments) and #225 through `gh
  issue view`.
- **Numbers to re-check before a spec:** every *(estimate)*: the pixel sizes (they assume a 60°
  vertical field of view at 1440p), the band populations, the skinning traffic and milliseconds, the
  memory per person, the shadow-pose BLAS count and size, the variety probabilities, the appearance
  genome's bytes, the familiarity record's size and the bandwidth split.
