# Research — Large worlds: coordinates, partitioning, streaming, LOD, impostors, terrain

> Companion to the Forge architecture notes and to the prior in-house conclusions in
> `world/docs/SPEC.md` §4.1 (frame hierarchy, D-04) and `world/docs/research/nature-terrain.md`
> §6.4 (vegetation rendering). Written 2026-09-23. Every citation was checked against at least
> one reachable page that day; the exceptions and the things that could not be found are under
> [Checked and left out](#checked-and-left-out) and [Verification notes](#verification-notes).
> Continuous-LOD geometry (Nanite's DAG, meshlets, mesh shaders) is only pointed to here; the
> detail lives in `gpu-geometry.md`.

The owner asked four things: whether origin shifting is still how big worlds are done or whether
something better is known now; which coordinate convention to adopt; what the best spatial
partitioning is on CPU and GPU; and whether octahedral impostors are still the right tool for
trees. The short answer is that the prior project's choices — `f64` positions inside a hierarchy
of reference frames, camera-relative `f32` rendering, reversed-Z with an infinite far plane, an
equi-angular cube-sphere quadtree for planets and CDLOD morphing for the far field — are what the
strongest shipped systems converged on between 2003 (Dungeon Siege) and 2022 (Unreal Engine 5),
and nothing published since replaces them. What has moved is around the edges: GPU BVH
construction is now a single kernel (H-PLOC, 2024), hardware ray-tracing acceleration structures
have become a general spatial query structure usable from compute, concurrent binary trees give
GPU-driven terrain subdivision at planetary scale in a fraction of a millisecond, and Epic's
experimental Nanite Foliage (voxelised aggregates) is the first credible challenger to octahedral
impostors for *dense distant* vegetation, though not for the mid-distance band.

> **State of the art in five sentences.** Positions are stored in 64-bit (doubles, or integer
> cells plus a local float) inside nested reference frames, and rendering is done in 32-bit
> relative to the camera: Unreal Engine 5, Star Citizen, Space Engineers, Elite Dangerous, Godot's
> double builds, Unity HDRP and the Rust `big_space` crate all do a variant of this, and pure
> floating-origin rebasing survives only as a single-player trick that Epic itself now calls
> legacy. Depth is a 32-bit float reversed-Z buffer with an infinite far plane, which Reed's 2015
> measurements show has zero comparison error and which Outerra found matches a 24-bit
> logarithmic buffer without the early-Z penalty. Broad-phase and visibility structures are
> unglamorous and settled — layered AABB trees or loose octrees on the CPU, sorted uniform grids
> and Morton-ordered BVHs on the GPU — with the one genuinely new option being the hardware
> ray-tracing BVH used for gameplay and audio queries. Streaming is cell-based with proxy geometry
> for unloaded cells (World Partition + HLOD, object containers, Ghost of Tsushima's fine-grained
> streaming), textures stream through virtual texturing, and LOD selection is driven by
> screen-space error, with simulation LOD (significance) now a first-class sibling of render LOD.
> For trees, octahedral impostors remain the best cost/quality answer from roughly 100 m to 1 km,
> and the open question — which the Forge demo should measure — is whether a voxelised aggregate
> beats impostor clumps beyond that.

**Contents**

1. [Precision: who does what, and the mathematics](#1-precision-who-does-what-and-the-mathematics)
2. [Coordinate conventions](#2-coordinate-conventions)
3. [Spatial partitioning on the CPU](#3-spatial-partitioning-on-the-cpu)
4. [Spatial partitioning on the GPU](#4-spatial-partitioning-on-the-gpu)
5. [Streaming and world partition](#5-streaming-and-world-partition)
6. [LOD: theory and practice](#6-lod-theory-and-practice)
7. [Impostors and distant vegetation](#7-impostors-and-distant-vegetation)
8. [Terrain for planets and big flat worlds](#8-terrain-for-planets-and-big-flat-worlds)
9. [Precision in physics and networking](#9-precision-in-physics-and-networking)
10. [Recommendation for Forge](#recommendation-for-forge)
11. [Checked and left out](#checked-and-left-out)
12. [Verification notes](#verification-notes)

---

## 1. Precision: who does what, and the mathematics

The arithmetic first, because every design below is a response to it. An IEEE `f32` has a 24-bit
significand, so its spacing (one ULP) at magnitude *x* is 2^(⌊log₂ x⌋ − 23): about 8 mm at
100 km, 6 cm at 1,000 km, 1 m at 10,000 km. An `f64` (53 bits) is 15 µm at 100 Gm and 30 µm at
1 AU, but 2 m at one light-year. So doubles alone carry a star system comfortably (a 10¹³ m
Oort-cloud radius still resolves 2 mm) and fail at galactic scale, which is why every
galaxy-sized system pairs doubles with an integer grid of sectors. A depth buffer has the same
shape of problem along one axis, solved by reversed-Z.

**Christopher Thorne. "Origin-centric techniques for optimising scalability and the fidelity of
motion, interaction and rendering." PhD thesis, University of Western Australia, 2007.** [paper]
[foundational]
<https://research-repository.uwa.edu.au/en/publications/origin-centric-techniques-for-optimising-scalability-and-the-fide>
(conference version: Thorne, "Using a Floating Origin to Improve Fidelity and Performance of
Large, Distributed Virtual Worlds", *Cyberworlds* 2005, DOI 10.1109/CW.2005.94)

The thesis that named the technique. Thorne shows that positional error grows with distance from
the origin, that more bits alone do not cure it, and that centring computation on the viewer — a
stationary camera with the world moved around it — removes jitter, z-buffer tearing and
non-repeatable dynamics in one move; floating origin is one member of the "origin-centric"
family he generalises.
*Bearing:* the formal justification for camera-relative rendering; "move the world, not the
camera" is a principle, not a hack.

**Scott Bilas. "The Continuous World of Dungeon Siege." GDC, 2003.** [talk] [foundational]
<https://www.gdcvault.com/play/1022728/The-Continuous-World-of-Dungeon> (slides:
<https://www.gamedevs.org/uploads/the-continuous-world-of-dungeon-siege.pdf>)

The earliest shipped frame hierarchy and still its clearest statement. Dungeon Siege axed the
unified coordinate system: each terrain chunk ("Siege Node") is its own space, a position is
`SiegePos = node ID + (x, y, z)`, nodes link through "doors" that are transforms, and world space
exists for one frame only — a target node is chosen and a "space walk" accumulates transforms
outward from it, like a skeleton. The slide reads "There is no world space!"; the same structure
became the basis for streaming and culling.
*Bearing:* Forge's `(frame_id, local_position)` is this design; Bilas's advice to switch frames
at every node boundary, as often as possible, argues for eager rather than lazy re-parenting.

**Epic Games. "Large World Coordinates in Unreal Engine 5" and "Large World Coordinates
Rendering in Unreal Engine 5." Unreal Engine documentation, 2022–2026; with Unity Technologies,
"Camera-relative rendering", HDRP manual.** [docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/large-world-coordinates-in-unreal-engine-5>
· <https://dev.epicgames.com/documentation/en-us/unreal-engine/large-world-coordinates-rendering-in-unreal-engine-5>
· <https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/Camera-Relative-Rendering.html>

UE5 moved its core transforms to doubles on the CPU, raising `WORLD_MAX` from 21 km (UE4) to
88 million km (still marked beta). On the GPU it does *not* use doubles: shaders work in
"translated world space" (camera-relative), recommended over converting `WorldPosition` to
doubles because it "results in superior performance while retaining high precision"; absolute
positions, where unavoidable, are tile + offset (tile size 256k units) or a double-float pair
(`FDFVector`, documented as the more precise). Unity's HDRP does the same renderer-wide: every
object and light is translated by the negated camera position before any other transform, on by
default in `ShaderConfig.cs`, with `_WorldSpaceCameraPos` the one exception.
*Bearing:* the two largest engines validated the exact split Forge inherited — doubles on the
CPU, camera-relative floats on the GPU — and UE5's tile/double-float shader types are the model
for the few Slang shaders (planet-scale noise) that must see an absolute coordinate.

**Godot Engine. "Large world coordinates." Godot 4 documentation; and Clay John, "Emulating
Double Precision on the GPU to Render Large Worlds", godotengine.org, 17 October 2022.** [docs]
[web] [recent]
<https://docs.godotengine.org/en/stable/tutorials/physics/large_world_coordinates.html> ·
<https://godotengine.org/article/emulating-double-precision-gpu-render-large-worlds/>

Godot 4 offers a double-precision build (`Vector3` becomes 64-bit; official binaries do not ship
it), tabulates float precision against distance, and names origin shifting as the
single-precision alternative with a cost in multiplayer. The article explains the GPU side
without doubles: the model-view translation is split into a float and a float residual, rotation
and scale stay separate, and only the translation gets emulated precision — with the caveat that
custom vertex transforms and world-space fragment maths do not benefit (triplanar mapping
jitters far from the origin; particles snap).
*Bearing:* a compact statement of both options and their limits; Godot's list of what breaks is a
checklist of Forge shaders that must be camera-relative from the start.

**Cloud Imperium Games (Sean Tracy, interviewed by Steve Burke). "Star Citizen's Sean Tracy on
64-bit Engine Tech." GamersNexus, 30 September 2016; with Star Citizen Wiki, "Star Engine" and
"Object Container Streaming".** [web] [still-current]
<https://gamersnexus.net/gg/2622-star-citizen-sean-tracy-64bit-engine-tech-edge-blending> ·
<https://starcitizen.tools/Star_Engine> · <https://starcitizen.tools/Object_Container_Streaming>

CryEngine was converted to 64-bit positioning over about eight months; Tracy stresses that only
"physics and positioning" needed the change. The wiki, citing CIG's October 2014 monthly report,
records the other half: "Camera Relative Rendering" keeps the GPU in 32-bit at no cost, the Zone
System (deployed June 2015) and local physics grids let a player walk inside a moving ship, and
object containers (2016) became the streaming unit — client OCS in 2018, with a 2023 CitizenCon
figure of over 700,000 streamed-in entities per server.
*Bearing:* the closest shipped analogue to Forge's constructs-with-their-own-frame model,
including per-zone physics; the wiki is secondary, so pull the comm-links it cites before any
number enters a spec.

**Marek Rosa. "Space Engineers: Super-large worlds, Procedural asteroids and Exploration."
Marek Rosa dev blog, 17 December 2014.** [web] [still-current]
<https://blog.marekrosa.org/2014/12/space-engineers-super-large-worlds_17/>

Keen moved all game objects to doubles while leaving Havok in 32-bit by clustering the world into
independent physics clusters — minimum 20 km, typically 50–100 km, growing with dynamic-object
density — each object positioned relative to its cluster centre; the safe radius went from 10 km
to 10⁹ km (6.6 AU).
*Bearing:* the simplest published recipe for "f64 world, f32 physics": physics islands are spatial
clusters with their own origin, which is what Forge's per-construct spaces become when constructs
drift apart or dock.

**Doc Ross (Frontier Developments), interviewed by 80.lv. "Generating the Universe in Elite:
Dangerous." 80.lv, 5 April 2018.** [web] [still-current]
<https://80.lv/articles/generating-the-universe-in-elite-dangerous>

A single 64-bit integer encodes a sector's x, y, z, its layer in an eight-layer octree, the star
system within the sector and the body within the system — the galaxy is addressed, not
positioned. Surface generation needs millimetre precision on inputs of tens of billions of
millimetres, so Frontier wrote its noise libraries in doubles and again in "dual-float" for GPUs
without fast doubles.
*Bearing:* integer sector addressing with doubles inside a system is Forge's frame hierarchy; the
dual-float noise library is the pattern for planet-scale procedural evaluation in Slang.

**Peter Freese. "Solving Accuracy Problems in Large World Coordinates." In Andrew Kirmse (ed.),
*Game Programming Gems 4*, ch. 2.3, pp. 157–170. Charles River Media, 2004 (ISBN
1-58450-205-9).** [book] [foundational]
<http://www.gameenginegems.net/gemsdb/article.php?id=280> (the Gems database entry; the chapter
is not online)

The first published "far position": an integer segment index plus a float offset inside the
segment, a hybrid of fixed point (the segments) and floating point (the offset). A difference is
taken segment first, in integers, so the large parts cancel exactly and only the small offsets
meet in floating point. Added on 2026-09-25 at the owner's request.
*Bearing:* the pattern behind UE5's tile + offset and `big_space`'s cells below. It is the form
proposed for the GPU instance table (#93, a 🟡 amendment to D-004): `(int3 cell, float3 local)`,
with the camera-relative position computed cell first. A million GPU-placed instances then need no
per-frame rewrite and no doubles on the GPU.

**Aevyrie et al. `big_space` — floating origin and nested integer grids for Bevy. GitHub,
2022–2026.** [code] [recent]
<https://github.com/aevyrie/big_space>

A Rust/Bevy plugin that chunks space into nestable integer grids (`i8` to `i128`) with an `f32`
transform inside each cell, a floating origin and spatial hashing; nested grids give
"planet in a star system" hierarchies, and the README's table runs from a few solar-system widths
(`i32`) to far beyond the observable universe (`i128`). Release 0.12 targets Bevy 0.18, dual
MIT/Apache-2.0. (A 2024 Godot/C# write-up by Frozen Fractal reaches the same design — `f64`
global, `f32` relative to the player each frame — and reports that the double position had to
move into a custom ECS rather than onto scene nodes.)
*Bearing:* the nearest Rust implementation of Forge's model and evidence that integer-cell +
local-float is a live alternative to `f64`-inside-a-frame; the ECS lesson is organisational — the
double position belongs in the entity data model.

**Patrick Cozzi, Kevin Ring. *3D Engine Design for Virtual Globes*. A K Peters/CRC Press, 2011.**
[book] [foundational]
<https://www.virtualglobebook.com/> ·
<https://www.routledge.com/3D-Engine-Design-for-Virtual-Globes/Cozzi-Ring/p/book/9781568817118>

The GIS side of the same problem, by the people behind Cesium: chapters on vertex-transform
precision (rendering relative to centre, double-single "GPU RTE" emulation), depth-buffer
precision, geometry clipmapping and chunked LOD for whole-Earth data.
*Bearing:* the best single reference for the precision chapter of Forge's renderer, and the source
of the "relative to centre / relative to eye" vocabulary everyone else re-derived.

**Nathan Reed. "Depth Precision Visualized." NVIDIA Developer Blog / reedbeta.com, 15 July 2015;
with Brano Kemen, "Maximizing Depth Buffer Range and Precision", Outerra blog, November 2012.**
[web] [still-current]
<https://www.reedbeta.com/blog/depth-precision-visualized/> ·
<https://outerra.blogspot.com/2012/11/maximizing-depth-buffer-range-and.html>

Reed plots depth distributions for every common setup and simulates comparison error: a float
buffer with reversed-Z gives a *zero* error rate and "erases the distinctions" between
precomposed versus separate matrices and finite versus infinite far planes, so the refinements of
Upchurch & Desbrun (2012) stop mattering. Outerra, from the planet-renderer side, found a
standard buffer gives about four usable decades, a 24-bit logarithmic buffer about nine ("blades
of grass in front of your eyes" with objects hundreds of kilometres away) but costs early-Z when
written from the fragment shader, and that the reversed float buffer nearly matches it for free.
*Bearing:* settles depth for Forge: `D32_SFLOAT`, reversed-Z, infinite far plane,
greater-or-equal compare; no logarithmic-depth path.

---

## 2. Coordinate conventions

There is no consensus, only a table; each row was confirmed against the primary page.

| System | Handedness | Up | Forward | Units |
|---|---|---|---|---|
| glTF 2.0 | right | +Y | +Z is the front | metres |
| Vulkan clip/framebuffer | — | framebuffer Y **down**, depth 0..1 | — | — |
| Unreal Engine | left | +Z | +X | centimetres |
| Unity | left | +Y | +Z | metres |
| Godot 4 | right | +Y | −Z | metres |
| Blender | right | +Z | −Y (front view) | metres |
| USD | right | Y default, Z allowed (`upAxis`) | — | `metersPerUnit` |

**Khronos Group. "glTF 2.0 Specification", §3.4 Coordinate System and Units. Khronos Registry,
2017 (maintained).** [docs] [still-current]
<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#coordinate-system-and-units>

"glTF uses a right-handed coordinate system", +Y up, the front faces +Z, linear units are metres.
It is the interchange format every DCC tool exports and every Rust loader assumes.
*Bearing:* the strongest single argument for the engine's own convention: matching glTF makes
every import an identity transform.

**Sascha Willems. "Flipping the Vulkan viewport." saschawillems.de, 29 March 2019; with the
Vulkan specification, "Fixed-Function Vertex Post-Processing".** [web] [docs] [still-current]
<https://www.saschawillems.de/blog/2019/03/29/flipping-the-vulkan-viewport/> ·
<https://docs.vulkan.org/spec/latest/chapters/vertexpostproc.html>

Vulkan's framebuffer origin is top-left with Y down, and clip depth is 0..1 unless
`VK_EXT_depth_clip_control` asks for −1..1, so OpenGL-style projections render upside-down with
reversed winding. The fix is a negative `VkViewport::height` (from `VK_KHR_maintenance1`, core
since 1.1), which flips Y in the clip-to-framebuffer transform and leaves shaders untouched.
*Bearing:* Forge's `ash` backend sets a negative viewport height once and builds an infinite
reversed-Z projection for a 0..1 range; no per-shader Y negation.

**Epic Games. "Coordinate System and Spaces in Unreal Engine." Unreal Engine documentation; with
the Godot, Blender and OpenUSD manuals.** [docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/coordinate-system-and-spaces-in-unreal-engine>
· <https://docs.godotengine.org/en/stable/tutorials/3d/introduction_to_3d.html> ·
<https://docs.blender.org/manual/en/2.91/editors/3dview/navigate/viewpoint.html> ·
<https://openusd.org/release/api/group___usd_geom_up_axis__group.html> (Unity:
<https://discussions.unity.com/t/unity-is-a-left-handed-coordinate-system-why/11897>)

Unreal is left-handed, Z-up, X-forward, in centimetres; Unity is left-handed Y-up Z-forward;
Godot is right-handed Y-up, −Z forward, "1 unit being equal to 1 meter"; Blender is Z-up
("with the Z axis pointing upwards"); USD's `upAxis` defaults to Y with Z the only other legal
value, and exporters are "strongly encouraged" to write it.
*Bearing:* Z-up is the GIS/DCC habit and Y-up the interchange habit; on a planet "up" is a local
radial vector anyway, so the global convention matters only for import and flat test scenes.

---

## 3. Spatial partitioning on the CPU

The queries differ and so should the structures: visibility wants a static hierarchy per streamed
cell; physics broad-phase wants cheap incremental updates and layered static/dynamic sets;
gameplay wants fixed-radius neighbour search; audio wants ray casts against coarse occluders.

**Thatcher Ulrich. "Loose Octrees." *Game Programming Gems* 1, ch. 4.11, Charles River Media,
2000; and "Notes on spatial partitioning", tulrich.com.** [book] [web] [foundational]
<https://www.tulrich.com/geekstuff/partitioning.html>

An octree whose nodes are doubled in extent and kept concentric, so an object is placed by its
bounding radius (level) and centre (node) without straddling children; movers re-insert in O(1)
without allocation and there are no hotspot cells where small objects sink into huge nodes.
Ulrich shipped it in several VR titles and notes static geometry deserves a tighter structure.
*Bearing:* the structure for Forge's per-frame *dynamic* entity set (gameplay queries, coarse
visibility of movers); static chunk geometry goes in a BVH.

**Christer Ericson. *Real-Time Collision Detection*. Morgan Kaufmann, 2004; with Cohen, Lin,
Manocha, Ponamgi, "I-COLLIDE", *Symposium on Interactive 3D Graphics* 1995, 189–196.** [book]
[paper] [foundational]
<https://realtimecollisiondetection.net/> ·
<https://www.cs.princeton.edu/courses/archive/spr01/cs598b/papers/cohen95.pdf>

Ericson is the reference for the whole zoo — grids, BVHs and their heuristics, k-d trees, BSPs,
sort-and-sweep — with the robustness and cache-layout chapters papers skip. I-COLLIDE introduced
sweep-and-prune: sort AABB extents per axis, maintain the lists incrementally, and report pairs
overlapping on all axes; it exploits temporal coherence and degrades only when everything clumps.
*Bearing:* the book to keep open while writing the `spatial` crate; its robustness chapter is also
the argument for broad-phase in the frame's local `f32`. Sweep-and-prune is what Rapier's default
broad phase descends from, so Forge gets it per physics island.

**Jorrit Rouwé. "Architecting Jolt Physics for Horizon Forbidden West." GDC 2022; and the Jolt
architecture documentation.** [talk] [docs] [recent]
<https://jrouwe.nl/architectingjolt/ArchitectingJoltPhysics_Rouwe_Jorrit_Notes.pdf> ·
<https://jrouwe.github.io/JoltPhysics/>

Jolt's broad phase is a 4-wide AABB tree ("quad tree": four children so one SIMD op tests all),
one tree per broad-phase layer (at least static and dynamic), lock-free for concurrent query and
modification with background rebuilds; bodies of a streamed tile are built into a sub-tree on the
loading thread and grafted in. The motive was lock contention on PS5, not asymptotics.
*Bearing:* the modern shape of a broad phase for a streamed open world — layered trees, batch
insertion per streamed cell, concurrent queries from gameplay threads — requirements for whatever
physics crate Forge wraps.

**Ingo Wald, Solomon Boulos, Peter Shirley. "Ray Tracing Deformable Scenes Using Dynamic Bounding
Volume Hierarchies." *ACM Transactions on Graphics* 26(1), 2007.** [paper] [foundational]
DOI 10.1145/1189762.1206075

Made "refit, don't rebuild" respectable: a BVH built once with the surface-area heuristic and
refitted bottom-up each frame stays good enough for deforming and moving geometry, with a rebuild
only when quality decays.
*Bearing:* the policy for Forge's per-chunk BVHs, CPU and GPU alike — SAH build at stream-in,
refit for animated props and vegetation, rebuild on a quality threshold.

---

## 4. Spatial partitioning on the GPU

**Tero Karras. "Maximizing Parallelism in the Construction of BVHs, Octrees, and k-d Trees."
*High Performance Graphics* 2012; with Lauterbach et al., "Fast BVH Construction on GPUs",
*Computer Graphics Forum* 28(2), 2009; and Karras & Aila, "Fast Parallel Construction of
High-Quality Bounding Volume Hierarchies", HPG 2013.** [paper] [still-current]
<https://research.nvidia.com/sites/default/files/pubs/2012-06_Maximizing-Parallelism-in/karras2012hpg_paper.pdf>
· <https://mgarland.org/files/papers/gpubvh.pdf> ·
<https://research.nvidia.com/publication/2013-07_fast-parallel-construction-high-quality-bounding-volume-hierarchies>

The NVIDIA lineage. Lauterbach's LBVH sorts primitives by Morton code and builds the hierarchy
from the sorted order (hundreds of thousands of triangles in under 100 ms in 2009); Karras 2012
builds the whole binary radix tree in place and in parallel, from which BVHs, octrees and k-d
trees all fall out in one pass; Karras & Aila 2013 restructure fixed-size treelets bottom-up to
reach over 90 % of the best offline builder's trace performance at about 40 M triangles/s.
*Bearing:* Karras 2012 is the first GPU structure to implement (particles, foliage instances,
debris) because it is short; Morton ordering is also how Forge should order instances for culling
and streaming.

**Daniel Meister, Jiří Bittner. "Parallel Locally-Ordered Clustering for Bounding Volume
Hierarchy Construction." *IEEE TVCG* 24(3), 2018, 1345–1353; with Benthin et al., "PLOC++",
*Proc. ACM CGIT* 5(3), 2022.** [paper] [still-current]
<https://dcgi.fel.cvut.cz/publications/2018/meister-tvcg-ploc> ·
<https://www.intel.com/content/www/us/en/developer/articles/technical/ploc-for-bounding-volume.html>

Bottom-up agglomerative clustering with clusters kept in Morton order, merging a batch of mutually
nearest pairs per iteration; up to twice faster builds and 17 % faster traces than the 2013 state
of the art. PLOC++ reworks it for a single persistent kernel.
*Bearing:* the family Forge's GPU builder should belong to — SAH-quality trees directly, no
separate optimisation pass.

**Carsten Benthin, Daniel Meister, Joshua Barczak, Rohan Mehalwal, John Tsakok, Andrew Kensler.
"H-PLOC: Hierarchical Parallel Locally-Ordered Clustering for Bounding Volume Hierarchy
Construction." *High Performance Graphics* 2024 / *Proc. ACM CGIT* 7(3), 2024.** [paper] [recent]
<https://gpuopen.com/download/HPLOC.pdf> (DOI 10.1145/3675377; reference gist:
<https://gist.github.com/natevm/6618402427ad6466bf555d67602adfa8>)

Builds the whole binary BVH in a single kernel launch at PLOC++ quality, 1.1–3.6× faster overall
and 1.6–13× faster for the binary phase, plus a one-kernel conversion to wide BVHs; it is what AMD
ships in its driver builders.
*Bearing:* the current best answer to "rebuild a BVH on the GPU every frame" — Forge's fallback
acceleration structure without ray-tracing hardware, and its builder for gameplay-only proxies.

**Simon Green. "CUDA Particles" / "Particle Simulation using CUDA." NVIDIA CUDA SDK white paper,
2008–2013; with Teschner et al., "Optimized Spatial Hashing for Collision Detection of Deformable
Objects", *VMV* 2003, 47–54.** [web] [paper] [foundational]
<https://developer.download.nvidia.com/compute/cuda/2_2/sdk/website/projects/particles/doc/particles.pdf>
· <https://cgl.ethz.ch/Downloads/Publications/Papers/2003/Tes03/Tes03.abstract>

The canonical GPU uniform grid: hash each particle to a cell, radix-sort by cell, read per-cell
ranges from the sorted array (an atomics variant is given for comparison; sorting wins because
neighbours become memory-coherent); cell size equals the interaction radius. Teschner's spatial
hash compresses an unbounded grid into a table, so it needs no bounds and no memory for empty
space.
*Bearing:* Forge's structure for anything with a fixed query radius on the GPU — particles,
crowds, grass interaction — and, hashed, the index for unbounded space cells (debris, drifting
constructs) where an octree root would have to cover the whole system.

**Khronos Group. "Ray Tracing in Vulkan" (Koch, Hector, Barczak, Werness), December 2020, and
`VK_KHR_ray_query`; with Zellmann, Weier, Wald, "Accelerating Force-Directed Graph Drawing with
RT Cores", arXiv:2008.11235, 2020; and Valve's Steam Audio documentation.** [web] [docs] [paper]
[still-current]
<https://www.khronos.org/blog/ray-tracing-in-vulkan> ·
<https://docs.vulkan.org/refpages/latest/refpages/source/VK_KHR_ray_query.html> ·
<https://arxiv.org/abs/2008.11235> ·
<https://valvesoftware.github.io/steam-audio/doc/capi/radeon-rays.html>

Acceleration structures are a two-level hierarchy (BLAS of triangles or AABBs, TLAS of instances),
hardware-built and opaque; ray queries expose traversal "in any shader stage", compute included,
without a ray-tracing pipeline. Zellmann et al. reformulate fixed-radius neighbour search as ray
tracing so RT cores do the traversal, 4–13× faster than a CUDA software version. Steam Audio
already computes occlusion, reflections and reverb by ray tracing the game's geometry (built-in,
Embree, or Radeon Rays on the GPU at 50–150× single-thread speed for baking).
*Bearing:* the hardware BVH is a general spatial query structure: line of sight, sweeps, radius
queries, audio occlusion and reflections can all be ray queries from compute against the TLAS the
renderer already maintains, with AABB primitives for proxies; audio is a client of the same
structure, not the owner of another.

---

## 5. Streaming and world partition

**Epic Games. "World Partition" and "World Partition — Hierarchical Level of Detail". Unreal
Engine documentation, 2021–2026.** [docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/world-partition-in-unreal-engine> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/world-partition---hierarchical-level-of-detail-in-unreal-engine>

One persistent level partitioned at cook time into a runtime grid (example: 256 m cells, 768 m
loading range) loaded around *streaming sources*; data layers add non-spatial loading; every actor
is its own file (One File Per Actor) so many people can edit one map. HLOD layers generate
instanced, merged or simplified proxies per cell and "content in unloaded cells is replaced with
HLODs", so distant mountains and forests stay visible without their source data.
*Bearing:* the industry-default streaming architecture in 2026; Forge's equivalents are cube-sphere
tiles and space cells as the grid, constructs as object containers, and a generated proxy
(impostor or voxel aggregate) per unloaded cell.

**Adrian Bentley. "Zen of Streaming: Building and Loading 'Ghost of Tsushima'"; and Matthew
Pohlmann, "Samurai Landscapes: Building and Rendering Tsushima Island on PS4." GDC 2021.** [talk]
[recent]
<https://gdcvault.com/play/1027205/Zen-of-Streaming-Building-and> ·
<https://gdcvault.com/play/1027352/Samurai-Landscapes-Building-and-Rendering>

A world about 15× larger than the studio's previous game with fast loads and small patches, from
fine-grained streaming and per-system optimisations across terrain, pathing, physics, AI and
rendering; the companion talk covers millions of instances placed by a GPU-interpreted rule
language, vegetation density management, extended draw distance and runtime budgets on PS4.
*Bearing:* streaming granularity is a whole-engine discipline — every Forge system must stream
per cell — and the placement/LOD/budget tuning is the reference for a console-class forest.

**Johan Andersson. "Terrain Rendering in Frostbite Using Procedural Shader Splatting." SIGGRAPH
2007 course; with Jaap van Muijden, "GPU-Based Run-Time Procedural Placement in 'Horizon: Zero
Dawn'", GDC 2017.** [talk] [foundational] [still-current]
<https://www.ea.com/frostbite/news/terrain-rendering-in-frostbite-using-procedural-shader-splatting>
· <https://www.gdcvault.com/play/1024700/GPU-Based-Run-Time-Procedural>

Frostbite renders terrain as a quadtree with fixed grids in the leaves and computes materials
from compact masks in the shader rather than storing textures, so only masks stream and the
terrain can be destroyed and re-textured at run time. Guerrilla places vegetation, rocks, wildlife
and effects on the GPU around the player from artist-authored density rules as the world streams,
so placement data is never stored and a biome edit re-places everything.
*Bearing:* stream *inputs* — fields and rules — and evaluate on the GPU; the far-field aggregate in
§7 then becomes a generation output rather than an asset.

**Sean Barrett. "Sparse Virtual Textures." GDC 2008; J.M.P. van Waveren, "Software Virtual
Textures", id Software, 25 February 2012; and Epic's "Virtual Texturing" documentation.** [talk]
[paper] [docs] [foundational] [still-current]
<https://silverspaceship.com/src/svt/> ·
<https://www.mrelusive.com/publications/papers/Software-Virtual-Textures.pdf> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-texturing-in-unreal-engine>

Barrett's open reformulation of MegaTexture: a huge virtual texture, a page table, a pixel shader
that translates virtual to physical addresses, and a feedback pass that reports needed pages. Van
Waveren's production version from RAGE gives the numbers — 128×128 pages, a resident 4096² cache
(1024 pages) fronting a million-page virtual texture, a quadtree walk with graceful fallback to
coarser resident pages — and solutions for filtering, oversubscription, LOD snapping and
compression. UE5 adds the *runtime* variant whose texels are rendered on demand, the mechanism
behind landscape materials that rocks and trees sample.
*Bearing:* Forge's texture residency: a runtime VT cache of the evaluated procedural planet
material, streaming VT for authored construct textures, and van Waveren's fallback-to-coarser-page
behaviour so streaming hitches stay invisible.

**Brian Karis, Rune Stubbe, Graham Wihlidal. "A Deep Dive into Nanite Virtualized Geometry."
SIGGRAPH 2021 course *Advances in Real-Time Rendering in Games*.** [talk] [recent]
<https://advances.realtimerendering.com/s2021/index.html>

Cited here for its streaming half only: geometry is cut into fixed-size pages of clusters, pages
are requested by the GPU from what was visible, and a hierarchy is traversable while finer pages
are absent. The DAG, cluster selection and rasterisation are in `gpu-geometry.md`.
*Bearing:* the model for geometry residency — page-granular, GPU-requested, tolerant of missing
detail — the same shape as virtual textures.

---

## 6. LOD: theory and practice

**David Luebke, Martin Reddy, Jonathan D. Cohen, Amitabh Varshney, Benjamin Watson, Robert
Huebner. *Level of Detail for 3D Graphics*. Morgan Kaufmann, 2002.** [book] [foundational]
<http://lodbook.com/>

The textbook: discrete, continuous and view-dependent LOD; simplification operators and error
metrics; screen-space and perceptual criteria; temporal and terrain LOD. Older than the GPU-driven
era, but the error-metric chapters have not been superseded.
*Bearing:* the metric — projected geometric error in pixels — that every LOD decision in Forge
(terrain morph, mesh LOD, impostor switch, simulation LOD) should be expressed in.

**Peter Lindstrom, David Koller, William Ribarsky, Larry F. Hodges, Nick Faust, Gregory A.
Turner. "Real-Time, Continuous Level of Detail Rendering of Height Fields." SIGGRAPH 1996.**
[paper] [foundational]
DOI 10.1145/237170.237217

The first continuous heightfield LOD driven by a screen-space error threshold: vertices are
removed when their projected error falls under a pixel tolerance, with dependency rules keeping
the mesh continuous.
*Bearing:* the origin of "select by projected error, not by distance" — CDLOD and Nanite are both
descendants; Forge's terrain and geometry should share one error-to-pixels function.

**Michael Garland, Paul S. Heckbert. "Surface Simplification Using Quadric Error Metrics."
SIGGRAPH 1997; with Arseny Kapoulkine, meshoptimizer, GitHub, 2017–2026.** [paper] [code]
[foundational] [still-current]
<https://www.cs.cmu.edu/~garland/quadrics/quadrics.html> · <https://github.com/zeux/meshoptimizer>

Edge collapses ordered by a per-vertex quadric accumulating squared distance to the original
planes: fast, high quality, and the basis of every production simplifier since, including
meshoptimizer's — MIT-licensed, attribute-aware, seam-preserving with a permissive mode for
faceted meshes, with a README recipe for packing an LOD chain coarse-first into one index buffer.
The same library builds meshlets. (Hoppe's *Progressive Meshes*, SIGGRAPH 1996, is the
continuous-resolution ancestor and the origin of geomorphing.)
*Bearing:* Forge does not write its own simplifier; one dependency covers §6 and the meshlet
pipeline in `gpu-geometry.md`.

**Unity Technologies. "LOD Group" and "SpeedTree". Unity Manual.** [docs] [still-current]
<https://docs.unity3d.com/Manual/class-LODGroup.html> · <https://docs.unity3d.com/Manual/SpeedTree.html>

Levels selected by screen-relative height; transitions hard, cross-faded (dithered) over a
configurable width, or vertex-interpolated for SpeedTree models. SpeedTree assets arrive as an
LOD Group whose last level is a billboard that can face the light in shadow passes, and each tree
costs three to four materials.
*Bearing:* the two transition mechanisms Forge needs — dithered crossfade under TAA for meshes,
geomorph for terrain and trees — and the industry's default ladder (mesh LODs → cards →
billboard) that Forge extends with an octahedral and an aggregate band.

**Epic Games. "Significance Manager" and "Overview of Mass Entity." Unreal Engine
documentation.** [docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/significance-manager-in-unreal-engine>
· <https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-mass-entity-in-unreal-engine>

A central framework that scores every registered object's importance and lets it react — disable
emitters and audio, tick AI less often, drop animation quality — under per-category budgets; Mass
Entity stores per-chunk LOD data for the same purpose in its data-oriented crowd and traffic
simulation.
*Bearing:* simulation LOD as infrastructure: one significance score per entity that render LOD,
animation LOD, physics fidelity, AI tick rate and audio all read, or a living world will not scale
past a few hundred agents.

---

## 7. Impostors and distant vegetation

**Paulo W. C. Maciel, Peter Shirley. "Visual Navigation of Large Environments Using Textured
Clusters." *Symposium on Interactive 3D Graphics* 1995.** [paper] [foundational]
DOI 10.1145/199404.199420

The impostor idea: replace clusters of distant geometry by textured polygons pre-rendered from
representative views and select between geometry and impostors under a frame-time budget.
*Bearing:* still the definition; everything below is a way of choosing views and blending them.

**Xavier Décoret, Frédo Durand, François X. Sillion, Julie Dorsey. "Billboard Clouds for Extreme
Model Simplification." SIGGRAPH 2003.** [paper] [foundational]
DOI 10.1145/882262.882326

Approximates a model by a small set of textured planes chosen by optimisation in plane space, each
carrying colour and normals, so a few quads keep parallax far better than one billboard.
*Bearing:* the basis of card-based tree LODs and of "clumps"; the representation for the
40–150 m band before octahedral capture.

**Philippe Decaudin, Fabrice Neyret. "Volumetric Billboards." *Computer Graphics Forum* 28(8),
2009 (successor to "Rendering Forest Scenes in Real-Time", *Eurographics Symposium on Rendering*
2004).** [paper] [foundational]
<https://maverick.inria.fr/Publications/2009/DN09/>

A tree as a small volume texture rendered by GPU slicing: full parallax from any direction,
correct antialiasing at distance, no popping between view-dependent frames; the 2004 paper did
the same for whole forests with volumetric slabs over the terrain.
*Bearing:* the direct ancestor of Nanite Foliage's voxels — a volumetric aggregate for the far
band is a twenty-year-old idea that memory and fill-rate now allow.

**Ryan Brucks. "Octahedral Impostors." shaderbits.com, 2018, and the ImpostorBaker plugin; with
Amplify Creations, "Amplify Impostors", Unity asset.** [web] [code] [still-current]
<https://shaderbits.com/blog/octahedral-impostors> · <https://github.com/ictusbrucks/ImpostorBaker>
· <https://amplify.pt/unity/amplify-impostors/>

Captures the object from views distributed over an octahedron (or a hemi-octahedron for objects
only seen from above), packs them in one atlas with depth and normals, and blends the three
nearest frames at run time with per-pixel depth for parallax and stable lighting; the plugin
became Unreal's built-in impostor baker. Amplify's Unity implementation (spherical and octahedral,
instanced, lit, shadowed, depth-writing, one-click bake into LOD Groups) shows the technique is a
solved production feature.
*Bearing:* the technique the prior project chose and the one to keep for the mid band; Forge needs
the full octahedron because players fly.

**Epic Games. "Nanite Foliage" (Nanite Assemblies, Nanite Voxels, Nanite Skinning). Unreal
Engine 5.6–5.8 documentation, experimental.** [docs] [recent]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage>

Assemblies micro-instance repeated parts (one tree: 41 M triangles from 2,160 part instances,
3.5 GB to 29 MB on disk); *voxels* take over at pixel size — "triangles seamlessly switch to
voxels" — to render dense aggregate meshes without overdraw or LOD popping; skinning replaces
material wind. Epic claims 500k tree instances of dozens of variants; the feature is still
experimental.
*Bearing:* the first production-grade aggregate for distant vegetation. It does not replace
impostors at mid distance (pixel-sized voxels at 200 m are still many voxels per tree); it
replaces impostor *clumps* and canopy cards beyond that. Forge's own version is a voxelised or
volumetric forest layer generated per cell, and the demo should measure both.

---

## 8. Terrain for planets and big flat worlds

**Filip Strugar. "Continuous Distance-Dependent Level of Detail for Rendering Heightmaps
(CDLOD)." 2010 (paper, source and data on GitHub, MIT).** [paper] [code] [still-current]
<https://github.com/fstrugar/CDLOD>

A quadtree of fixed-resolution grid patches selected by true 3D distance to the observer, with
vertices morphed toward the coarser level in the vertex shader so there is no stitching and no
popping; the LOD function is the same over the whole mesh.
*Bearing:* the prior choice for the far field, confirmed: it maps directly onto cube-sphere
quadtree tiles, needs no seams between faces if the morph uses the same distance function, and is
trivially GPU-driven with indirect draws.

**Frank Losasso, Hugues Hoppe. "Geometry Clipmaps: Terrain Rendering Using Nested Regular
Grids." *ACM Transactions on Graphics* 23(3), 2004; Asirvatham & Hoppe, "Terrain Rendering Using
GPU-Based Geometry Clipmaps", *GPU Gems 2* ch. 2, 2005; and Huw Bowles, "Crest: Novel Ocean
Rendering Techniques in an Open Source Framework", SIGGRAPH 2017 Advances course.** [paper] [book]
[talk] [foundational] [still-current]
<https://hhoppe.com/proj/geomclipmap/> ·
<https://developer.nvidia.com/gpugems/gpugems2/part-i-geometric-complexity/chapter-2-terrain-rendering-using-gpu-based-geometry>
· <https://advances.realtimerendering.com/s2017/index.html>

Nested regular grids centred on the viewer, updated with toroidal addressing as the viewer moves,
elevation in vertex textures (grid size 2^k − 1, typically 255), a compressed pyramid (a 40 GB US
heightmap at 100:1) and fractal synthesis for the finest levels. Crest's open-source ocean is
this plus CDLOD-style morphing, with a displacement texture per level and shape and shading
layered into the same cascade.
*Bearing:* the alternative to CDLOD for *flat* worlds and the right structure for the ocean mesh;
on a planet the clipmap must sit on the local tangent plane of the camera's cube-sphere cell and
curve with height — a few lines in the vertex shader.

**Matt Zucker, Yosuke Higashi. "Cube-to-sphere Projections for Procedural Texturing and Beyond."
*Journal of Computer Graphics Techniques* 7(2), 2018.** [paper] [still-current]
<https://jcgt.org/published/0007/02/01/>

Compares approximately equal-area cube-to-sphere warps — identity, tangent (equi-angular),
Everitt's, optimised odd polynomials, COBE — on area distortion and GPU cost, with GLSL. Verdict:
the fifth-order odd polynomial if speed is paramount, COBE if area preservation is the criterion,
and the tangent warp "to maximize ease of implementation, as it outperforms Everitt's method in
preserving area". (The cubed sphere itself is Ronchi, Iacono & Paolucci, *J. Comput. Phys.*
124(1), 1996, DOI 10.1006/jcph.1996.0047.)
*Bearing:* confirms the equi-angular warp — analytic inverse, near-uniform cells — and names the
drop-in upgrade (5th-order polynomial) if per-vertex `tan`/`atan` ever shows in a profile.

**Google. "S2 Cell Hierarchy." S2 Geometry developer guide.** [docs] [still-current]
<https://s2geometry.io/devguide/s2cell_hierarchy>

Six cube faces projected to the sphere with a non-linear warp, each a quadtree of 30 levels
(leaf cells about 1 cm on Earth), ordered by a Hilbert curve so nearby cells have nearby 64-bit
ids (face bits, two bits per level, a sentinel).
*Bearing:* the persistence and streaming key for planet tiles: one `u64` per cell that sorts
spatially and yields parent/child by bit operations; the Hilbert linking is what keeps cube-face
seams spatially coherent.

**Anis Benyoub, Jonathan Dupuy. "Concurrent Binary Trees for Large-Scale Game Components."
*High Performance Graphics* 2024 (arXiv:2407.02215); building on Dupuy, "Concurrent Binary Trees
(with application to longest edge bisection)", HPG 2020.** [paper] [recent]
<https://arxiv.org/abs/2407.02215> · <https://onrendering.com/>

A GPU-resident binary tree encoded as a bitfield with sum reduction, driving longest-edge
bisection entirely on the GPU; the 2024 paper extends it from square domains to arbitrary
half-edge meshes and makes the tree a memory manager, rendering "planetary scale geometry out of
very coarse meshes" in under 0.2 ms on console hardware with mesh shaders.
*Bearing:* the GPU-driven successor to CDLOD's CPU quadtree — one structure per planet face or
construct hull, subdivided by projected error each frame with no CPU round trip. A spike after the
CDLOD path is proven.

**Tao Ju, Frank Losasso, Scott Schaefer, Joe Warren. "Dual Contouring of Hermite Data." SIGGRAPH
2002, 339–346; Schaefer, Ju, Warren, "Manifold Dual Contouring", *IEEE TVCG* 13(3), 2007; and
Eric Lengyel, the Transvoxel algorithm (PhD dissertation, UC Davis, 2010).** [paper] [foundational]
[still-current]
<https://www.cs.rice.edu/~jwarren/papers/dualcontour.pdf> ·
<https://people.engr.tamu.edu/schaefer/research/index.html> · <https://transvoxel.org/>

Dual contouring contours a signed octree whose edges carry exact intersections and normals,
placing one vertex per cell by minimising a quadric so sharp features survive and the octree
simplifies adaptively; the 2007 paper fixes its non-manifold outputs, and the 2002 paper already
shows real-time destructive edits. Transvoxel's transition cells stitch voxel meshes of different
resolutions without cracks, with complete tables and no patent claims.
*Bearing:* the mesher and the seam solution for Forge's near-field volumetric chunks (overhangs,
caves, edits), including the seam between the volumetric near field and the heightfield far field.

**Alex Evans. "Learning from Failure: a Survey of Promising, Unconventional and Mostly Abandoned
Renderers for 'Dreams PS4'." SIGGRAPH 2015 Advances course.** [talk] [still-current]
<https://advances.realtimerendering.com/s2015/index.html>

A whole world edited as signed distance fields stored in bricks, and years of experiments (sphere
tracing, voxel bricks, point splatting) with honest accounts of why each was abandoned.
*Bearing:* the reference for storing player edits and 3D features as SDF bricks per chunk, and a
catalogue of dead ends to avoid.

**Éric Bruneton, Fabrice Neyret, Nicolas Holzschuch. "Real-time Realistic Ocean Lighting using
Seamless Transitions from Geometry to BRDF." *Computer Graphics Forum* 29(2), 2010, 487–496; with
Proland (INRIA, BSD-3).** [paper] [code] [still-current]
<https://proland.inrialpes.fr/publications.html> · <https://proland.inrialpes.fr/>

Waves are geometry near the camera and are progressively folded into an analytic BRDF with
distance, so a planet-scale ocean shades consistently from the beach to the horizon seen from
orbit; Proland is the planet renderer (terrain to whole planets, atmosphere, ocean, rivers,
forests, Earth data at 500 m/90 m) that ships it.
*Bearing:* the missing piece of the water plan: the clipmap mesh handles the near field and this
transition keeps the same ocean correct at 100 km and from orbit without a second water system.

---

## 9. Precision in physics and networking

**Sébastien Crozet et al. Rapier — `rapier3d-f64`. crates.io, 2020–2026.** [code] [still-current]
<https://crates.io/crates/rapier3d-f64>

The Rust physics engine the prior project wrapped ships an `f64` variant of the same crate
(0.35.x in 2026, Apache-2.0), so a frame-level integrator and a construct-local `f32` world can
share code.
*Bearing:* construct-local physics in `f32` (the Space Engineers / Star Citizen model) and the
orbital integrator in `f64` from one dependency, behind one trait so Jolt stays swappable.

**Glenn Fiedler. "Snapshot Compression." gafferongames.com, 4 January 2015.** [web]
[still-current]
<https://gafferongames.com/post/snapshot_compression/>

Positions bounded to a known range and quantised (512 steps per metre, about 2 mm), orientations
as smallest-three quaternions (29 bits), delta-encoded against an acknowledged baseline: from
17 Mbit/s to about 256 kbit/s.
*Bearing:* quantise relative to the entity's frame and cell, never a global origin — Forge's
snapshot is `(frame_id, cell, i32 offset)` with the same 1–2 mm step.

**coherence. "World Origin Shifting." coherence documentation; with Epic Games, "World
Composition" (world origin shifting), Unreal Engine documentation.** [docs] [recent]
[still-current]
<https://docs.coherence.io/manual/advanced-topics/big-worlds/world-origin-shifting> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/world-composition-in-unreal-engine>

coherence's replication server stores absolute positions while each client has its own floating
origin and stores positions relative to it, reconciled on the wire; sub-millimetre accuracy is
claimed to 5 billion km, with callbacks for non-networked objects and cameras. UE4's origin
rebasing, by contrast, added an offset to every actor, and its page states that "world origin
shifting is not supported in the multiplayer games"; World Composition is now legacy, replaced by
World Partition and LWC.
*Bearing:* the closing argument on the owner's first question — the engine that made rebasing
mainstream retired it because of multiplayer — and the pattern for Forge's co-op: authority holds
`(frame, f64)`, every client renders relative to its own camera, the transport converts.

**Cloud Imperium Games. "Server Meshing and Persistent Streaming Q&A." Comm-Link 18397,
10 November 2021.** [web] [recent]
<https://robertsspaceindustries.com/en/comm-link/transmission/18397-Server-Meshing-And-Persistent-Streaming-Q-A>
(full text mirror: <https://star-citizen.wiki/Comm-Link:18397/en>)

Static meshing first (fixed assignment of object containers to server nodes), dynamic later; a
separate replication layer owns entity state; within a shard an entity has "one server node that
controls the entity, and multiple other server nodes that have a client view"; state persists in
a graph database and player items stow/unstow across shards.
*Bearing:* the only public design for distributing a frame-hierarchy world across servers; co-op
does not need meshing, but authority-per-entity and container-per-node should shape the netcode
so it can grow.

---

## Recommendation for Forge

**Coordinate model.** Keep D-04 and sharpen it. Every entity stores `(frame_id, position: f64x3,
orientation: f32 quat, velocity: f64x3)`; frames form a tree (galaxy sector `i64x3` grid → star
system → body → construct), each frame's origin is expressed in its parent as `f64`, and no frame
ever holds a magnitude above ~10¹³ m, so `f64` resolves ≤ 2 mm everywhere. Rendering, physics,
animation, audio and gameplay queries run in `f32` relative to an *anchor*: the camera for
rendering (translated world space, Unity/UE5/CIG style), the construct origin for physics (Space
Engineers clusters), the cell origin for GPU structures. Frame changes happen eagerly at
boundaries (Bilas), and shaders never see an absolute position except through one documented
double-float type for planet-scale procedural evaluation (Elite's dual-float, UE5's `FDFVector`).
This is the best known way in 2026; the alternatives — pure origin rebasing (retired by Epic for
multiplayer) and integer-cell-plus-`f32` (`big_space`) — are respectively worse for co-op and
equivalent in precision with more bookkeeping. Depth: `D32_SFLOAT`, reversed-Z, infinite far
plane, `GREATER_OR_EQUAL`.

**Up axis.** Right-handed, +Y up, −Z forward, metres — glTF's and Godot's convention. Every asset
arrives that way and the Rust ecosystem assumes it; on a planet the global axis is meaningless
and local frames are built from the radial vector, so the choice only affects import and flat
scenes; Vulkan's Y-down framebuffer is handled once by a negative viewport height. GIS heightfields
and Blender-native Z-up data get a fixed swap at import; cube-sphere face frames carry their own
tangent basis.

**Partitioning, per use.**
- *Static world geometry (visibility):* cube-sphere quadtree tiles on bodies and an octree of
  space cells, each holding a SAH BVH over its chunks/meshlets, culled GPU-side
  (`gpu-geometry.md`); S2-style `u64` cell ids as streaming and persistence keys.
- *Physics broad phase:* one physics space per construct and per terrain cluster (`f32`) with the
  wrapped engine's layered tree/SAP; the `f64` integrator for free bodies off rails.
- *Gameplay queries:* a loose octree per frame for movers, a spatial hash for unbounded space
  cells, and on RT hardware ray queries against the renderer's TLAS (line of sight, sweeps, radius
  queries); audio consumes the same structure.
- *GPU particles, crowds, foliage interaction:* Green's sorted uniform grid; H-PLOC when a real
  BVH is needed without RT hardware.

**LOD and impostor ladder for trees** (a 20–30 m tree at 1440p, 90° FOV; bands switch on
projected height in pixels, the metres are the corresponding distances):
1. **0–40 m** — full mesh LOD0/1 with skinned wind and per-branch pivots.
2. **40–150 m** — meshoptimizer LOD chain (3 levels) with dithered crossfade, then billboard-cloud
   cards for the last level.
3. **150 m–1 km (tree < ~40 px)** — full octahedral impostor, 16×16 frames with depth and normals,
   three-frame blend; hemi-octahedral only for ground-only assets. This is where octahedral is
   still the best known answer.
4. **1–6 km** — an *aggregate* per cell: a voxelised/volumetric forest layer (Nanite Foliage's
   voxels, Decaudin & Neyret's volumetric billboards) generated with the placement rules, not
   per-tree impostors. The open question; measure it.
5. **Beyond, and from orbit** — canopy as a terrain material and height layer.

**Terrain.** Equi-angular cube-sphere quadtree (Zucker & Higashi's tangent warp; 5th-order
polynomial if it ever costs), CDLOD morphing for the heightfield far field, dual contouring with
Transvoxel transitions for the volumetric near field, SDF bricks per chunk for edits and features;
materials and vegetation streamed as fields and rules through a runtime virtual-texture cache;
water as a tangent-plane clipmap near the camera folded into Bruneton's BRDF at distance. CBT on
mesh shaders is the scheduled successor experiment for the far field.

**The demo that proves it — "Relay".** Two players in co-op. One stands in a forest of one million
placed trees on a 1,500 km planet 10¹¹ m from its star; the other lands a ship beside them, both
walk aboard while it is moving, fly to a moon, land and walk out. Pass criteria: measured position
jitter under 1 mm at every stage (log the ULP of the anchor-relative position), no z-fighting on a
rig spanning 0.1 m to 10⁷ m, frame budget held (streaming, BVH refit and impostor updates each
under their allotted ms), and a toggle at 1 km that switches band 4 between impostor clumps and
the voxel aggregate with overdraw and memory recorded for both.

---

## Checked and left out

Kept so the bibliography is auditable: things looked for and not above, with the reason.

- **Sean Barrett, "Making a Big World in Unity"** (or any Barrett floating-origin talk) — does not
  exist as far as three searches could tell; his real GDC talk is *Sparse Virtual Textures* (2008),
  §5. The Unity floating-origin material that exists is community code and the coherence docs.
- **An official Epic talk dedicated to Large World Coordinates** — not found. The Unreal Fest 2022
  list mentions a large-worlds tools session but nothing LWC-specific, and the session page
  refused fetching (403); the two documentation pages in §1 are Epic's primary material.
- **Chris Thorne, "Rotation Floating Origin"** (ResearchGate, 2021) — appears in search results
  but the page refused fetching; not cited.
- **Elite Dangerous / Dual Universe as fixed-point examples** — neither is. Elite uses 64-bit
  *integer ids* for addressing and doubles/dual-floats for positions (§1); Dual Universe ran on
  Unigine 2's 64-bit engine (Wikipedia, citing Phoronix and PCInvasion) and shut down in August
  2025. Unigine's own double-precision page renders only through JavaScript and could not be
  read, so there is no Unigine entry.
- **Kerbal Space Program's Krakensbane** (the 0.17 floating-origin change) — the official wiki
  sits behind an Anubis anti-bot wall and the Fandom mirror returned HTTP 402; only Steam
  community threads were reachable, which are not citation-grade. KSP is mentioned only as the
  popular example.
- **"Massive Crowd on Assassin's Creed Unity: AI Recycling" (GDC 2015)** and a **Horizon
  Forbidden West vegetation talk** — remembered as existing, not confirmable once the search
  budget ran out (Bing and Mojeek returned nothing usable, DuckDuckGo served CAPTCHAs).
  Simulation and animation LOD is covered by Epic's Significance Manager and Mass Entity instead.
- **Enshrouded (Keen Games) voxel-engine talk** — no published talk or article could be located;
  the voxel entries in §8 cover the technique.
- **xraxra's "IMP" octahedral impostor baker** — the GitHub account exists but its repository
  list (GitHub API) has no impostor project; Brucks' plugin and Amplify are cited instead.
- **Neural impostors** — an arXiv search returns only *Neural Impostor: Editing Neural Radiance
  Fields with Explicit Shape Manipulation* (2023), a NeRF editing paper, not a vegetation
  technique. Nothing here is production-ready.
- **Ronchi, Iacono & Paolucci 1996** (the cubed sphere) — the DOI resolves but ScienceDirect and
  ADS refused fetching; mentioned inside the Zucker & Higashi entry only.
- **Decaudin & Neyret 2004, "Rendering Forest Scenes in Real-Time"** — DOI
  10.2312/EGWR/EGSR04/093-102 resolves to the Eurographics library, which refused the connection;
  folded into the verified 2009 entry.
- **Upchurch & Desbrun 2012, "Tightening the Precision of Perspective Rendering"** and **Hoppe
  1996, "Progressive Meshes"** — the former seen only through Reed's post, the latter confirmed on
  Hoppe's page but folded into the Garland entry to keep the list within budget.

---

## Verification notes

Checked on 2026-09-23 with WebSearch and WebFetch only; no browser pane and no YouTube pages were
used. Talks were confirmed on GDC Vault session pages or the *Advances in Real-Time Rendering*
course index pages (2015, 2017, 2021). WebSearch covered the first forty-odd lookups until the
session budget ran out; the rest was verified by direct fetches of canonical pages, the Semantic
Scholar API (rate-limited after a burst; it confirmed Lindstrom 1996, Maciel & Shirley 1995 and
Wald 2007), the arXiv and GitHub APIs, doi.org resolution, and local `pdftotext` extraction from
PDFs the fetcher saved: Bilas 2003, van Waveren 2012, Garland & Heckbert 1997, Ju et al. 2002,
Zucker & Higashi 2018, Rouwé 2022, Green 2008.

- **Confirmed from the primary page:** Thorne 2007 (UWA repository); both UE5 LWC pages (21 km →
  88 million km, tile + offset and double-float types); Unity HDRP camera-relative; Godot LWC docs
  and Clay John 2022; GamersNexus/Tracy 2016; starcitizen.tools *Star Engine* and *OCS*; Marek
  Rosa 2014; 80.lv/Doc Ross 2018; `big_space` (0.12 / Bevy 0.18); Frozen Fractal 2024; Cozzi &
  Ring (Routledge and book site); Reed 2015; Outerra 2012; glTF §3.4; Willems 2019 and the Vulkan
  spec; Epic coordinate page; Godot 3D intro; Blender 2.91 manual; USD `upAxis`; Ulrich's page;
  Ericson's site; I-COLLIDE PDF; Rouwé GDC 2022 notes and Jolt docs; Karras 2012 PDF; Karras &
  Aila 2013 (NVIDIA); Meister & Bittner (DCGI) and PLOC++ (Intel); H-PLOC (GPUOpen); Green (NVIDIA
  PDF); Teschner (ETH abstract); Khronos ray-tracing blog and `VK_KHR_ray_query`; Zellmann 2020
  (arXiv); Steam Audio docs; UE World Partition and HLOD; Bentley and Pohlmann 2021 (GDC Vault);
  Andersson 2007 (EA page); van Muijden 2017 (GDC Vault); Barrett SVT page and GDC Vault 417; van
  Waveren PDF; UE Virtual Texturing; Karis 2021 and Evans 2015 (Advances indexes); lodbook.com;
  Hoppe's geometry-clipmap and PM pages; GPU Gems 2 ch. 2; Garland CMU page; meshoptimizer; Unity
  LOD Group and SpeedTree pages; Significance Manager and Mass Entity; Decaudin & Neyret 2009
  (Maverick); ImpostorBaker; Amplify page; Nanite Foliage (5.8 docs); CDLOD repo; Zucker &
  Higashi (PDF text); S2 guide; Benyoub & Dupuy 2024 (arXiv) and onrendering.com; Ju 2002 and
  Schaefer 2007 (TAMU list); transvoxel.org; Proland site and publications page; Crest (Advances
  2017); `rapier3d-f64` (crates.io API); Fiedler 2015; coherence docs; UE World Composition; CIG
  comm-link 18397 (the wiki mirror carries the text; the RSI page returned only its title).
- **Weaker confirmations, stated plainly.** Thorne 2005: title and year from Wikipedia's
  *Floating origin* references and the DOI resolving to IEEE document 1587542; the IEEE page was
  not fetched. Décoret 2003: the DOI resolves to the ACM page, but ACM DL, HAL (Anubis) and
  Durand's publication page (404) all failed; authors/venue are from memory, consistent with the
  DOI. Lauterbach 2009: bibliographic data from the search result (CGF 28(2), 375–384); the
  Garland-hosted PDF was listed, not opened. Luebke et al.: publisher and ISBN from lodbook.com;
  the year 2002 is from memory (Google Books API rate-limited). Brucks 2018: shaderbits.com
  returned only its tag list, so the technique description is from the prior project's notes and
  the plugin; the URL is live. Proland's publications page prints the 2012 forests paper's volume
  as 29(2) (Eurographics 2012 is CGF 31(2)), so that paper is not cited by volume. Unity's
  left-handed convention was confirmed only from Unity community pages; the Transform and rotation
  manual pages fetched do not state handedness.
- **Blocked for the record:** ACM DL, Wiley, jcgt.org HTML (its PDF was fetchable), HAL and the
  KSP wiki (Anubis), Eurographics diglib (connection refused), ScienceDirect, ADS, IEEE,
  ResearchGate, GDC Vault search, the Unreal Fest session page, DuckDuckGo (CAPTCHA), Mojeek (403),
  Bing (locale-mangled results for quoted queries), web.archive.org (not permitted by the
  fetcher), Semantic Scholar after roughly ten calls (429).
- **Added on 2026-09-25 (#93):** Freese 2004. The title, chapter 2.3, the pages, the editor, the
  publisher and the ISBN come from the Gems database entry and the Library of Congress table of
  contents. The method ("far positions", a segment plus an offset) comes from search results and
  from Godot issue #18136, which cites it; the Ogre forum thread on it did not load. The chapter
  itself was not read, so nothing beyond the segment-first difference is cited from it.
- **Numbers to re-check before they enter a spec:** UE5's shader tile size (256k units in the 5.8
  docs; it has changed between 5.x releases), Star Citizen's "700,000 entities" (secondary
  source), Space Engineers' cluster sizes (2014 blog), and the H-PLOC speed-ups (paper figures on
  AMD hardware).
