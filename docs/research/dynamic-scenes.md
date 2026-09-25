# Research — Dynamic scenes: moving geometry in a GPU-driven, ray-traced renderer

> Companion to `gpu-geometry.md` (the cluster pipeline and the two-pass occlusion), `lighting-gi.md`
> (the probes and the ray-tracing tiers) and `large-worlds.md` §1 (the integer cells the instance
> table now uses). Written 2026-09-25 for issue #79, moving geometry, with #69 (probes woken when
> something moves), #95 (more async overlap) and #80 (the space battle whose first step is ships on
> paths) read alongside. Every citation was checked that day against a reachable page or, where the
> network proxy refused the host, against the search engine's record of it; the distinction is kept
> per entry under [Verification notes](#verification-notes), and what could not be found is under
> [Checked and left out](#checked-and-left-out).

Everything Forge draws today is static: the belt, the city's million instances, a TLAS built once,
probes that settle for good. The owner wants instances whose transforms change every frame — ships
on paths in the belt, cars in the city's streets — sharp under TAA and DLSS, with shadows and probes
that follow and the static captures untouched. The short answer from the published pipelines: keep a
previous transform per instance and upload what changed; two-pass occlusion needs no repair for
movers as long as the second pass tests against a pyramid built this frame, so "movers go to pass 2"
is correct and cheap at ten thousand movers among a million statics; motion vectors come from the
visibility buffer by re-projecting the same triangle through the previous transform, which is all
DLSS asks for; the vendors say to *build* a top-level structure rather than update it and that its
cost grows with the instance count, so a million instances need a second, small structure for the
movers (or NVIDIA's partitioned TLAS), never a rebuild of the whole; the probes already see movers
through their rays, and what needs a wake is the classification Forge froze after eight updates; and
#95's double-buffering is what lets the probe update, the mover TLAS build and the wake run during
the previous frame's tail.

> **State of the art in five sentences.** A GPU-driven scene keeps its instances in a device table
> that the CPU patches with the records that changed, each carrying its previous transform
> (Unreal's GPU Scene; Nanite's instance culling against the previous frame's HZB with the previous
> transforms), and the first of the two occlusion passes draws what the previous frame's depth
> showed while the second catches what it missed against this frame's pyramid, an algorithm Aaltonen
> and Haar shipped in 2015 and Nanite reproduced. Motion vectors are written per pixel from the
> object's previous position, dilated to the nearest depth, and consumed by a history-validated
> temporal filter (Karis 2014, Pedersen 2016, the 2020 survey); DLSS takes that buffer at render
> resolution with the jitter kept out of the matrices. Ray-tracing vendors say to rebuild the
> top-level structure each frame rather than refit it, that its cost is proportional to the instance
> count, that bottom-level refits are for limited deformation, and, since 2025, NVIDIA offers a
> partitioned TLAS whose global partition holds the movers so that a scene of a million statics
> rebuilds only what moved. Probe GI absorbs movers through its rays with a hysteresis of about
> 97 % (100 ms to converge), lowers the hysteresis near a fast or large change, re-classifies probes
> from fixed rays, and, in the production SDK, measures its own variability to stop tracing when
> settled; Lumen instead throttles a surface cache fed by Nanite captures. Async compute pays when
> the overlapped passes stress different units (rays and texture reads beside a raster-bound draw),
> loses when both are bandwidth- or export-bound, and was worth 5–10 % on a shipped AMD title and
> nothing on NVIDIA's of the time, which is why every overlap is measured pass by pass.

**Contents**

1. [Dynamic instances in GPU-driven pipelines](#1-dynamic-instances-in-gpu-driven-pipelines)
2. [Motion vectors and temporal techniques with movers](#2-motion-vectors-and-temporal-techniques-with-movers)
3. [Acceleration structures with moving instances](#3-acceleration-structures-with-moving-instances)
4. [Probe-based GI and moving geometry](#4-probe-based-gi-and-moving-geometry)
5. [The movers themselves: paths, traffic, determinism](#5-the-movers-themselves-paths-traffic-determinism)
6. [Async compute overlap](#6-async-compute-overlap)
7. [Recommendation for Forge](#recommendation-for-forge)
8. [What the numbers say](#what-the-numbers-say)
9. [Checked and left out](#checked-and-left-out)
10. [Verification notes](#verification-notes)

---

## 1. Dynamic instances in GPU-driven pipelines

The pipelines below keep the scene on the GPU and let the CPU touch only what changed. What differs
is how much state per instance they keep across frames and how the two occlusion passes treat an
instance whose position last frame is not its position now.

**Ulrich Haar (Ubisoft Montréal), Sebastian Aaltonen (RedLynx). "GPU-Driven Rendering Pipelines."
SIGGRAPH 2015, *Advances in Real-Time Rendering in Games*; with Aaltonen's note on the algorithm,
X, June 2021.** [talk] [web] [foundational] [still-current]
<https://advances.realtimerendering.com/s2015/> ·
<https://x.com/SebAaltonen/status/1402954450281578501>

The course page lists the talk's parts — motivation, mesh cluster rendering, the pipeline overview,
occlusion depth generation — for Assassin's Creed Unity's per-material instance batching and
RedLynx's clean-slate compute pipeline (`gpu-geometry.md` §1). Aaltonen's 2021 note states the
occlusion scheme in one sentence: "Use previous frame data as a starting point for the first pass
and then fill missing clusters in the second pass. RenderDoc captures show that Nanite is using the
same algorithm."
*Bearing:* Forge's two passes are this scheme (`shaders/meshlet.slang`, #33). Its correctness with
movers rests on one property: the second pass tests against a pyramid built *this* frame, so an
object the previous depth hid wrongly (a mover, or something a mover has uncovered) is drawn this
frame, one pass later, never a frame later. A mover can cost wasted work, never a wrong pixel; the
design question is which pass it takes.

**Graham Wihlidal (Frostbite). "Optimizing the Graphics Pipeline with Compute." GDC 2016.** [talk]
[foundational] [still-current]
<https://www.gdcvault.com/play/1023109/Optimizing-the-Graphics-Pipeline-With>

Frostbite's compute triangle filtering, "how the compute power of the console and PC GPUs can be
used to improve the triangle throughput beyond the limits of the fixed function hardware", built
with AMD (GeometryFX, open source); write-ups of the talk record that async compute lets the culling
shaders "run almost for free by overlapping compute and rasterization workloads".
*Bearing:* the first shipped statement that the culls belong beside the raster on the other queue;
§6 takes it up.

**Brian Karis, Rune Stubbe, Graham Wihlidal (Epic Games). "A Deep Dive into Nanite Virtualized
Geometry." SIGGRAPH 2021, *Advances in Real-Time Rendering in Games*; with Epic Games, "Nanite
Virtualized Geometry", Unreal Engine 5.8 documentation.** [talk] [docs] [foundational]
[still-current]
<https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-virtualized-geometry-in-unreal-engine>

Nanite culls instances, then the clusters of visible instances, in two passes: the first tests
against the HZB of the previous frame using the previous frame's transforms; this frame's HZB is
then built, used for the second pass, and updated after it (the previous-transform detail is from a
course summary of the talk, §10). The documentation states the dynamic-object contract: Nanite
"supports dynamic translation, rotation, and non-uniform scaling of meshes, whether it is dynamic or
static", limited "to transformations that can be expressed in a single 4x3 matrix multiply,
uniformly applied to the entire mesh". Write-ups record that pixel velocity "is written for
transform-based movement (either the object moving or the camera moving)" in the material pass.
*Bearing:* the production answer to this file's first two questions. Rigid movers need a transform
and its previous value, nothing more; pass 1 may test a mover where it *was*, which keeps big movers
as occluders at the price of reading the previous transform in the cull. Forge can start with the
simpler rule (§7) and adopt Nanite's once the previous-transform table exists anyway.

**Epic Games. "Mesh Drawing Pipeline in Unreal Engine" (GPU Scene). Unreal Engine 5.8
documentation; with the `FPrimitiveSceneData` layout as documented by third-party write-ups.**
[docs] [web] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/mesh-drawing-pipeline-in-unreal-engine>

"Supporting platforms use GPUScene to upload primitive data to a scene-wide buffer (UpdateGPUScene)
and index into it with a PrimitiveId." The per-primitive record holds `LocalToWorld`,
`WorldToLocal`, `PreviousLocalToWorld` and `PreviousWorldToLocal`, and the update gathers the
primitives marked dirty during the frame (transform, material, mesh or visibility changes) into an
upload buffer that a pass scatters into the scene buffer (secondary write-ups, §10).
*Bearing:* the model for Forge's mover update: a list of changed records, one compute pass to apply
it, the previous transform kept beside the current one on the GPU. Unreal keeps two full matrices
per primitive; Forge's 80-byte cell record cannot, hence §7's small table for the movers only.

**Jalal Eddine El Mansouri (Ubisoft Montréal). "Rendering 'Rainbow Six | Siege'." GDC 2016.**
[talk] [still-current]
<https://www.gdcvault.com/play/1023287/Rendering-Rainbow-Six-Siege>

A GPU-driven pipeline built for "massively and procedurally destructible levels": material-based
draw calls, culling at several levels, checkerboard rendering, 60 fps across platforms. The culling
table gives 10 537 unbatched draws against 412 batched (visibility, G-buffer, decals) and 64 for
shadows, a "culling efficiency" of 73 %; the trade write-up's summary is that GPU-driven rendering
"is the reason Rainbow Six Siege can have thousands of dynamic rubble objects created from its
destruction systems", and the slides note the culling's cost is hidden "on consoles using async
jobs".
*Bearing:* the shipped proof that a GPU-driven instance table absorbs thousands of movers without a
CPU draw per object; Forge's ten thousand ships or cars are the same order. (id Software's Doom
Eternal talk, SIGGRAPH 2020, lists "geometry caches" among its dynamic-world systems: the same
pattern of motion authored as data and the GPU handed this frame's transform.)

---

## 2. Motion vectors and temporal techniques with movers

Forge's motion vectors come from the depth and the two cameras: right for everything that does not
move, wrong for everything that does, which TAA and DLSS then smear. The fix has one source of truth
— where the shaded point was last frame — and the visibility buffer makes it cheap.

**Brian Karis (Epic Games). "High Quality Temporal Supersampling." SIGGRAPH 2014, *Advances in
Real-Time Rendering in Games*.** [talk] [foundational]
<https://advances.realtimerendering.com/s2014/>

The talk that made TAA the industry's default: jittered samples accumulated into a history
reprojected by motion vectors, the history clamped to the neighbourhood of the current frame so that
stale colour cannot survive. Its slides were not re-read here (§10); its role is the origin.
*Bearing:* Forge's `Taa` is this design. Movers add no new filter, only correct input: the vector of
a moving pixel must be the object's motion, or the clamp is all that stands between the viewer and a
ghost.

**Lasse Jon Fuglsang Pedersen (Playdead). "Temporal Reprojection Anti-Aliasing in INSIDE." GDC
2016; with Playdead's `temporal` repository (Unity implementation and slides).** [talk] [code]
[still-current]
<https://www.gdcvault.com/play/1022970/Temporal-Reprojection-Anti-Aliasing-in> ·
<https://github.com/playdeadgames/temporal>

The complete recipe in the open: the frustum jittered with "the first 16 samples of Halton(2,3)", "a
velocity buffer from camera motion and dynamics", "reprojection using velocity based on the closest
depth fragment" (the 3 × 3 depth dilation that keeps a thin mover's edge attached to its motion),
"neighbourhood clipping to the RGB min-max of a 3x3 region, and a motion blur fallback".
*Bearing:* the two details Forge's motion pass needs once objects move: the velocity buffer covers
"dynamics", not just the camera, and the reprojection reads the velocity of the *nearest* pixel in a
3 × 3 window, so a ship's silhouette drags its own vector rather than the background's.

**Lei Yang, Shiqiu Liu, Marco Salvi. "A Survey of Temporal Antialiasing Techniques." *Computer
Graphics Forum* 39(2) (Eurographics 2020 State of the Art Reports), 607–621.** [paper]
[still-current]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.14018> (DOI 10.1111/cgf.14018; author copy
<http://behindthepixels.io/assets/files/TemporalAA.pdf>)

TAA defined as "temporally-amortized supersampling" with two components, "sample accumulation and
history validation", the catalogue of its failures (ghosting where the history is wrong, blur where
the validation is too loose, flicker where it is too tight, disocclusion where no history exists),
and the same formulation extended to temporal upsampling.
*Bearing:* the vocabulary for the demo's checks. A ship without object motion vectors is a
history-validation failure the clamp partly hides; with them it is a disocclusion problem at the
trailing edge, which should show in the captures as a one-frame edge, not a trail.

**Christopher A. Burns, Warren A. Hunt (Intel). "The Visibility Buffer: A Cache-Friendly Approach
to Deferred Shading." *Journal of Computer Graphics Techniques* 2(2), 2013, 55–69; with John Hable,
"Visibility Buffer Rendering with Material Graphs", filmicworlds.com, July 2021.** [paper] [web]
[foundational] [still-current]
<https://jcgt.org/published/0002/02/04/> ·
<https://filmicworlds.com/blog/visibility-buffer-rendering-with-material-graphs/>

The G-buffer "replaced with a simple visibility buffer that only stores a triangle index and
instance ID per sample, encoded in as few as four bytes"; every attribute is reconstructed from the
triangle afterwards. Hable's frame breakdown lists the motion-vector pass among those that follow
the visibility resolve, next to the shadow pass, TAA and tonemapping.
*Bearing:* the reason per-object motion is cheap in Forge: the resolve already fetches the pixel's
three vertices and its barycentrics (`ARCHITECTURE.md` §4). The previous clip position of the same
surface point is those object-space vertices through the instance's previous transform and the
previous camera; the motion pass needs one extra transform per mover and nothing per static.

**NVIDIA. Streamline SDK, "DLSS Programming Guide" (`docs/ProgrammingGuideDLSS.md`), 2.x, 2026
(MIT).** [docs] [code] [still-current]
<https://github.com/NVIDIA-RTX/Streamline/blob/main/docs/ProgrammingGuideDLSS.md>

"DLSS requires depth, motion vectors, render-res input color and final-res output color buffers."
The motion vectors are tagged `kBufferTypeMvec`; "if motion vector values in your buffer are in
{-1,1} range then motion vector scale factor in common constants should be {1,1}", and if they are
"in pixel space then scaling factors `sl::Constants::mvecScale` should be {1 / render width, 1 /
render height}"; "All SL matrices are row-major and should not contain any jitter offsets", and
"jitter offset values are in pixel space". The sentences read do not distinguish camera motion from
object motion.
*Bearing:* nothing changes in `DlssUpscaler` when objects move; what changes is the buffer Forge
already tags, and the same buffer serves TAA, so the demo's ghosting check is one capture per
resolve.

---

## 3. Acceleration structures with moving instances

Forge builds one TLAS over every instance at start (D-029): a million and one records written by a
compute pass, 12 ms once. Movers make it a per-frame question, and the vendors have written down the
rules.

**Khronos Group. Vulkan specification, "Acceleration Structures" (`VK_KHR_acceleration_structure`,
`VK_KHR_ray_query`); with nvpro-samples, `vk_raytracing_tutorial_KHR`, "Animation" chapter.**
[docs] [code] [still-current]
<https://github.com/KhronosGroup/Vulkan-Docs/blob/main/chapters/accelstructures.adoc> ·
<https://github.com/nvpro-samples/vk_raytracing_tutorial_KHR/blob/master/ray_tracing_animation/README.md>

An update (`VK_BUILD_ACCELERATION_STRUCTURE_MODE_UPDATE_KHR`, allowed only on a structure built with
`ALLOW_UPDATE_BIT`) is a refit: "the application is required to provide a full description of the
acceleration structure, but is prohibited from changing anything other than instance definitions,
transform matrices, and vertex or AABB positions"; it must not "change the number of geometries or
instances in the structure". Each instance carries "an 8-bit visibility mask"; "the instance may
only be hit if `Cull Mask & instance.mask != 0`". The tutorial shows the refit path:
`ALLOW_UPDATE_BIT_KHR` on the TLAS ("absolutely needed, since otherwise the TLAS cannot be
updated"), `ALLOW_UPDATE | PREFER_FAST_BUILD` on an animated BLAS, and "always update the TLAS when
BLAS are modified. This will make sure that the TLAS knows about the new bounding box sizes."
*Bearing:* the two levers Forge has without a vendor extension. A refit keeps the tree's topology,
so a mover that crosses the scene degrades it; a rebuild is a new tree at full cost; the mask is a
per-ray filter, not a way to skip the rebuild. Adding or removing a mover is a rebuild by rule,
which is what a small dedicated structure makes cheap; the refit flags are the owner's A/B (#79).

**NVIDIA. "Tips and Tricks: Ray Tracing Best Practices", NVIDIA Technical Blog, 2019 (from the GDC
2019 presentation); Juha Sjöholm, "Best Practices for Using NVIDIA RTX Ray Tracing (Updated)",
NVIDIA Technical Blog, 2020, since revised; with "RTX Memory Utility" (RTXMU), GitHub (MIT).**
[web] [code] [still-current]
<https://developer.nvidia.com/blog/rtx-best-practices/> ·
<https://developer.nvidia.com/blog/best-practices-for-using-nvidia-rtx-ray-tracing-updated/> ·
<https://github.com/NVIDIAGameWorks/RTXMU>

The 2019 post: "Build the Top-Level Acceleration Structure (TLAS) rather than Update. It's just
easier to manage in most circumstances, and the cost savings to refit likely aren't worth
sacrificing quality of TLAS." The updated post on the bottom level: "BLAS updates are a good choice
after limited deformations as they are significantly cheaper than rebuilds, however large
deformations after the previous rebuild can lead to non-optimal ray-trace performance"; compaction
pays "for updateable geometry with long lifetime", but "for fully dynamic geometry rebuilt every
frame, there's generally no benefit". RTXMU packages compaction and suballocation: "compaction is
proven to reduce the total memory footprint by more than a half".
*Bearing:* the policy as written: the TLAS that holds movers is *built* every frame, never refitted;
BLASes stay static (a ship or a car is rigid, so its BLAS is a cut like any prop's; refits are for
Phase 3's tumbling rocks); compaction is a later memory pass over the static BLASes (278 MiB in the
city), separate from this issue.

**AMD. "RDNA Performance Guide" (ray tracing section), GPUOpen, maintained; with "Improving
raytracing performance with the Radeon Raytracing Analyzer (RRA)", GPUOpen.** [docs] [web]
[still-current]
<https://gpuopen.com/learn/rdna-performance-guide/> ·
<https://gpuopen.com/learn/improving-rt-perf-with-rra/>

"Using fewer instances positively impacts TLAS build time"; "there is a trade-off between tighter
fit BLASes and longer TLAS build time due to more instances", so "always measure the impact";
"minimizing instance overlap and empty space gives the driver opportunity to make more optimal
acceleration structures", and "instance transforms that significantly stretch or skew the underlying
BLAS are often not optimal since BLASes are built relative to the non-deformed mesh".
*Bearing:* the cross-vendor half of the policy (#67). A mover TLAS of ten thousand tight, uniformly
scaled instances is the good case on both vendors; a million-instance structure rebuilt per frame is
the bad case on both.

**Epic Games. "Ray Tracing Performance Guide in Unreal Engine" and "Lumen Technical Details" (far
field), Unreal Engine 5.8 documentation.** [docs] [still-current]
<https://dev.epicgames.com/documentation/unreal-engine/ray-tracing-performance-guide-in-unreal-engine>
· <https://dev.epicgames.com/documentation/unreal-engine/lumen-technical-details-in-unreal-engine>

"Hardware Ray Tracing requires rebuilding the Top Level Acceleration Structure (TLAS) every frame.
This cost is proportional to the number of instances you need to include in this acceleration
structure." "Dynamically deforming meshes, like skinned meshes, also incur a large cost to update
the Ray Tracing acceleration structures each frame, proportional to the number of skinned
triangles." Lumen's far field is a second set of instances with its own range: enabled by
`r.LumenScene.FarField=1`, built from World Partition's HLOD1 meshes, marked per component with "Ray
Tracing Far Field", "traced beginning at the Max Trace Distance (default is 200m)" to a default of
one kilometre.
*Bearing:* Epic pays the per-frame rebuild because its scenes hold thousands of instances, not a
million; the far field is the split it uses to keep the near structure small. Whether it is a
separate TLAS in the engine could not be confirmed from the pages read (§10).

**NVIDIA / Khronos Group. "VK_NV_partitioned_acceleration_structure" (extension proposal), Vulkan
Documentation, 2025; with the `vk_partitioned_tlas` sample (nvpro-samples) and NVIDIA's RTX Mega
Geometry announcement.** [docs] [code] [recent]
<https://github.com/KhronosGroup/Vulkan-Docs/blob/main/proposals/VK_NV_partitioned_acceleration_structure.adoc>
· <https://github.com/nvpro-samples/vk_partitioned_tlas> ·
<https://developer.nvidia.com/blog/nvidia-rtx-mega-geometry-now-available-with-new-vulkan-samples>

The problem statement: "the current Top Level Acceleration Structure (TLAS) API necessitates a full
rebuild of the entire data structure even when only a few instances are modified, which does not
leverage temporal consistency across frames, especially in scenarios where most of the scene remains
unchanged." A PTLAS builds a structure per partition and combines them; when instances change, "any
partition that contains at least one of the affected instances will have their internal acceleration
structure rebuilt". A *global partition* has "an independent size limit and, during the build
process, instances in the global partition are treated as if they were in individual partitions",
meant for frequently updated instances. The sample shows "170000 dominoes on a board made of 1.2
million static objects" and calls the global partition "noticeably faster than the systematic
partition update, but may result in a slight loss of trace performance".
*Bearing:* exactly Forge's shape — a million statics, ten thousand movers — solved in the driver, on
NVIDIA only. It is the third option in §7, loaded only when the extension exists (the cross-vendor
rule), after the portable two-structure scheme is measured.

**Jakub Boksanský, Michael Wimmer, Jiří Bittner. "Ray Traced Shadows: Maintaining Real-Time Frame
Rates." In *Ray Tracing Gems* (Haines, Akenine-Möller, eds.), Apress, 2019, ch. 13, 159–182.**
[book] [still-current]
<https://link.springer.com/chapter/10.1007/978-1-4842-4427-2_13> (author copy
<https://boksajak.github.io/files/RTG1_RayTracedShadows.pdf>)

Ray-traced shadows made "a viable alternative to rasterization for real-time applications" by
spending rays where they matter: "the computation focuses on image regions where shadows actually
appear, in particular on the shadow boundaries", with an adaptive sample count and a temporal
filter.
*Bearing:* Forge's soft shadows are TAA averaging eight Vogel points over eight frames (D-029, #54);
a mover breaks that cycle at its penumbra, which is why the ballad already keeps hard shadows in
motion. Movers get hard shadows (one ray) unless the city's captures show the smear is acceptable; a
denoiser is Phase 4's.

**Ubisoft Montréal. "Ray tracing the world of Assassin's Creed Shadows." SIGGRAPH 2025, *Advances
in Real-Time Rendering in Games*; with Ubisoft, "Assassin's Creed Shadows Tech Q&A", 2025.** [talk]
[web] [recent]
<https://advances.realtimerendering.com/s2025/content/Advances%202025%20-%20Raytracing%20the%20world%20of%20Assassin's%20Creed%20Shadows.pdf>
· <https://www.ubisoft.com/en-us/game/assassins-creed/news/4XbPPtFyQEtIMWrA9xVDmZ/assassins-creed-shadows-tech-qa>

The most recent shipped case of ray tracing in a GPU-driven open world where almost everything
moves: "all vegetation in AC Shadows is physically animated on the GPU based on a dynamic wind
system driven by a fluid simulation", and the GI "uses a per-pixel raytracing pass using probe
volumes as a cache for secondary hit GI"; the Q&A's argument for ray-traced GI is that it "adapts to
changes in the scene", which "is particularly beneficial for a game with destructible objects and a
changing environment". The talk's structure-build numbers were not readable here (§10).
*Bearing:* a scene whose geometry deforms every frame still ships on the same two-level structures;
the design that tolerates it puts the movers' *direct* effect in per-pixel rays and keeps the probes
for the second bounce, the division Forge's resolve and probes already have.

---

## 4. Probe-based GI and moving geometry

Forge's probes (D-036) settle in two senses: their irradiance blends at 97 % a frame, and their
place and active state freeze after eight updates because free-running classification flipped
between two surfaces. Movers touch both. The literature answers the first with hysteresis rules and
the second with re-classification and, in production, a measure of change.

**Zander Majercik, Jean-Philippe Guertin, Derek Nowrouzezahrai, Morgan McGuire. "Dynamic Diffuse
Global Illumination with Ray-Traced Irradiance Fields." *Journal of Computer Graphics Techniques*
8(2), 2019; with McGuire, Majercik, Marrs, "Dynamic Diffuse Global Illumination" (articles, parts 3
and 6), GitHub, 2019.** [paper] [web] [foundational] [still-current]
<https://jcgt.org/published/0008/02/01/> ·
<https://github.com/morgan3d/articles/blob/main/2019-04-01-ddgi/overview.html> ·
<https://github.com/morgan3d/articles/blob/main/2019-04-01-ddgi/optimization.html>

The method Forge implements. The authors' own write-up says where movers stand: "Dynamic geometry is
the most difficult case for probes. There is no place to put the probes where dynamic geometry can't
intersect them during gameplay", and shows a hundred tumbling beach balls for which "DDGI correctly
computes the visibility at every frame" with "no shadow leaks". The blend "uses a hysteresis value
so that new and old data are combined"; "hysteresis values of 90% to 99.5% (of the previous frame)
are viable; lower gives faster update but can flicker. We recommend 97% hysteresis for the general
case". The production notes give the rates — "192 to 256 rays per probe per frame with setting
hysteresis to 95%" makes "lighting converge in about 100 ms at 60 Hz even in dramatic cases" — and
the rules for change: "Drop hysteresis either globally or for nearby probes when a large object is
moving *very* fast", the case being an object that "can cross an entire probe grid cell in a single
frame", and "on large change of lights, time of day, explosion, set piece destruction, or other
gameplay triggered change".
*Bearing:* the irradiance side needs no wake: the rays see the mover every frame and the 97 % blend
follows it in about a tenth of a second. What Forge adds for #69 is the paper's own advice as a
rule: lower the hysteresis of the probes a fast mover passes (§7).

**Zander Majercik, Adam Marrs, Josef Spjut, Morgan McGuire. "Scaling Probe-Based Real-Time Dynamic
Global Illumination for Production." *Journal of Computer Graphics Techniques* 10(2), 2021.**
[paper] [still-current]
<https://jcgt.org/published/0010/02/01/> (preprint arXiv:2009.10796)

The production extensions: probe relocation out of geometry, classification of probes that cannot
contribute so that their rays are skipped, the view bias in the lookup, and infinitely scrolling
volumes for open worlds. Forge's cascades, relocation, eight-update settling and bias are from this
paper (D-036).
*Bearing:* classification is the piece #69 is about. In the paper it runs continuously; Forge froze
it after eight updates because probes between two surfaces flipped every frame. The wake is
therefore Forge-specific: re-open the window for the probes a mover touched, not for everything.

**NVIDIA. RTXGI-DDGI SDK 1.3.x, `docs/DDGIVolume.md` and `ChangeLog.md`. GitHub (NVIDIA RTX SDKs
licence).** [docs] [code] [still-current]
<https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/docs/DDGIVolume.md> ·
<https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/ChangeLog.md>

The reference implementation's rules. Relocation moves a probe found inside geometry by its
back-face hits, at most "45% of the grid cell distance to prevent probes from being relocated
outside of their grid voxel". Classification, from fixed rays, disables probes "stuck inside
geometry", in "spaces without nearby geometry" or "far enough outside the play space". *Probe
variability* (added in 1.3.5) measures "an average coefficient of variation across the volume's
probes" to estimate "how volatile the volume's estimate of the light field is from one update to the
next"; the "variability value may not ever reach zero", probes "eventually settle in a state where
the variability stays within a given range", and then the "low frequency noise can be avoided by
pausing probe ray tracing and blending updates" — "the SDK exposes the measured variability and
expects the application to make decisions". Updates "may be scheduled at a lower frequency than the
frame rate, or even as asynchronous workloads that execute continuously on lower priority background
queues". The document gives "no specific guidance regarding dynamic geometry changes within a
stationary volume": the SDK re-traces and re-classifies every active probe on every update.
*Bearing:* the wake list of §7 is the cheap replacement for "re-classify everything every frame",
given Forge's freeze; variability is the measured "settled" signal #95's probe cadence needs; and
the SDK's blessing of low-priority background updates argues for keeping the probe passes on the
compute queue.

**Epic Games. "Lumen Technical Details" and "Lumen Performance Guide", Unreal Engine 5.8
documentation; with Daniel Wright, Krzysztof Narkowicz, Patrick Kelly, "Lumen: Real-time Global
Illumination in Unreal Engine 5", SIGGRAPH 2022, *Advances in Real-Time Rendering in Games*.**
[docs] [talk] [still-current]
<https://dev.epicgames.com/documentation/unreal-engine/lumen-technical-details-in-unreal-engine> ·
<https://dev.epicgames.com/documentation/unreal-engine/lumen-performance-guide-for-unreal-engine> ·
<https://advances.realtimerendering.com/s2022/index.html>

The comparison. Lumen keeps a *surface cache* of the scene's lighting: it "relies on Nanite's Level
of Detail (LOD) and Multi-View rasterization for fast scene captures to maintain the Surface Cache,
with all operations throttled to prevent hitches from occurring"; the Lumen Scene "operates on the
world around the camera", 200 m by default with software tracing and up to 800 m. The 2022 talk
covers "software ray tracing with signed distance fields, virtualized surface caching, hardware ray
tracing, final gathering".
*Bearing:* Lumen's answer to movers is a budgeted, throttled cache that re-captures what changed,
plus a screen-space final gather that sees this frame's geometry directly. Forge's equivalent is the
capped wake list; the lesson is the throttle.

---

## 5. The movers themselves: paths, traffic, determinism

Only what the renderer's demos need: ships that follow paths convincingly and cars that follow lanes
without colliding, from a seed, the same on the client and the server. City life at large is #90;
physics is Phase 3.

**Craig W. Reynolds. "Flocks, Herds, and Schools: A Distributed Behavioral Model." *Computer
Graphics* 21(4) (SIGGRAPH '87), 25–34; and "Steering Behaviors for Autonomous Characters." GDC
1999, Miller Freeman Game Group, 763–782.** [paper] [talk] [foundational] [still-current]
<https://www.red3d.com/cwr/papers/1987/boids.html> (DOI 10.1145/37402.37406) ·
<https://www.red3d.com/cwr/steer/gdc99/index.html>

Boids: "an elaboration of a particle system, with the simulated birds being the particles", each "an
independent actor that navigates according to its local perception", the flock's motion emerging
from separation, alignment and cohesion. The 1999 paper catalogues the individual behaviours — "seek
and flee; pursue and evade; wander; arrival; obstacle avoidance; containment; wall following; path
following; and flow field following" — as steering forces on a simple vehicle.
*Bearing:* the belt's ships are Reynolds' vehicles: path following along a spline with a lookahead,
separation from neighbours and asteroids, a formation as offset pursuit of a leader; forces on a
point mass integrated at the fixed tick, deterministic if the neighbour queries are ordered.

**Wenping Wang, Bert Jüttler, Dayue Zheng, Yang Liu. "Computation of Rotation Minimizing Frames."
*ACM Transactions on Graphics* 27(1), Article 2, 2008.** [paper] [still-current]
<https://dl.acm.org/doi/10.1145/1330511.1330513> (DOI 10.1145/1330511.1330513; author copy via
Microsoft Research)

"The double reflection method, which uses two reflections to compute each frame from its preceding
one to yield a sequence of frames to approximate an exact RMF", the frame used for "sweep or
blending surface modeling, motion design and control in computer animation and robotics".
*Bearing:* a ship's orientation along a spline. The Frenet frame flips at inflections and spins
where the curvature vanishes; the rotation-minimising frame does not, so the hull's up vector is
stable and banking is a roll proportional to the lateral acceleration on top of it.

**Martin Treiber, Ansgar Hennecke, Dirk Helbing. "Congested traffic states in empirical
observations and microscopic simulations." *Physical Review E* 62, 2000, 1805–1824; with Arne
Kesting, Martin Treiber, Dirk Helbing, "General Lane-Changing Model MOBIL for Car-Following
Models", *Transportation Research Record* 1999, 2007, 86–94.** [paper] [foundational]
[still-current]
<https://link.aps.org/doi/10.1103/PhysRevE.62.1805> (arXiv cond-mat/0002177) ·
<https://journals.sagepub.com/doi/10.3141/1999-10> (author copy
<https://www.mtreiber.de/publications/MOBIL_TRB.pdf>)

The Intelligent Driver Model: a single-lane, continuous car-following model that reproduces the
congested states observed on German freeways "near road inhomogeneities, specifically lane closings,
intersections, or uphill gradients". MOBIL derives lane changes for any car-following model from
"the utility of a given lane and the risk associated with lane changes", both "determined in terms
of longitudinal accelerations".
*Bearing:* the city's cars need one equation (IDM) per car per tick, against the car ahead in its
lane and a virtual stopped car at a red light; MOBIL only if the streets get two lanes a direction.

**Epic Games. "City Sample Project Unreal Engine Demonstration" (Mass Traffic, ZoneGraph), Unreal
Engine 5.8 documentation.** [docs] [still-current]
<https://dev.epicgames.com/documentation/unreal-engine/city-sample-project-unreal-engine-demonstration>

The City Sample "uses multiple spawners, one each for crowds, intersections, traffic, and parked
vehicles"; the ZoneGraph is "a lightweight design-driven flow for AI that follows a point-by-point
corridor structure and can store meaningful tags (static and dynamic)", with driving vehicles,
parked vehicles and crowds on separate lanes; entities are defined by traits "such as visuals, level
of detail, behaviors and more".
*Bearing:* the shape of the city demo's data: a lane graph derived from the street grid (D-028's
layer map already knows where the streets are), spawners at its edges, a per-entity LOD trait. How
the sample despawns vehicles was not confirmed from the page (§10); §7 states Forge's own rule.

**Glenn Fiedler. "Fix Your Timestep!" (2004); "Deterministic Lockstep" (2014); "Floating Point
Determinism" (2010). gafferongames.com.** [web] [foundational] [still-current]
<https://gafferongames.com/post/fix_your_timestep/> ·
<https://gafferongames.com/post/deterministic_lockstep/> ·
<https://gafferongames.com/post/floating_point_determinism/>

The simulation advances by a fixed `dt` from an accumulator, and rendering interpolates between the
last two states with an alpha equal to the remainder over `dt`. Lockstep sends "only the inputs that
control that system rather than the state", which works only if the simulation is deterministic, and
"floating point determinism across platforms is hard".
*Bearing:* D-016 already pins the arithmetic. The movers add the tick: 60 Hz fixed, the same seed on
client and server, transforms interpolated on the CPU when the frame's mover array is written.

---

## 6. Async compute overlap

#77 moved the sky tables and the probe update to the compute queue and gained 0.11 ms on the city's
frame while the probe zones stretched from 0.60 to 1.35 ms beside the geometry; the per-pass queue
flag whose waits the graph derives is Frostbite's FrameGraph design (O'Donnell, GDC 2017, whose
listed benefits include "simplified async compute"; `task-system.md`). #95 asks for more overlap
through double-buffering. The published guidance says what overlaps and what does not.

**Jonas Meyer (IO Interactive). "Rendering 'Hitman' with DirectX 12." GDC 2016 (Advanced Graphics
Techniques Tutorial Day).** [talk] [still-current]
<https://www.gdcvault.com/play/1023129/Advanced-Graphics-Techniques-Tutorial-Day>

Async compute "was used for screen space anti aliasing, screen space ambient occlusion and the
calculations for the light tiles"; press coverage of the talk records the gain as 5–10 % on AMD GPUs
and none on NVIDIA's of the time, and the developers' verdict that it was hard to tune.
*Bearing:* the sober number. Forge's 0.11 ms on a 2.3 ms frame is 5 %, in the same range; #95's
double-buffering may add a similar amount, not a multiple of it.

**NVIDIA. "Advanced API Performance: Async Compute and Overlap." NVIDIA Technical Blog, 2021.**
[web] [still-current]
<https://developer.nvidia.com/blog/advanced-api-performance-async-compute-and-overlap/>

"The general principle behind async compute is to increase the overall unit throughput by reducing
the number of unused warp slots and to facilitate the simultaneous use of nonconflicting datapaths."
"If a barrier or WFI is unavoidable and causes a throughput hole, filling the hole with async
compute is an effective solution"; "SM Idle % without conflicting high throughput units is almost
always a guaranteed improvement"; "be conscious of which asynchronous compute and graphics workloads
can be scheduled together. Use fences to pair up the right workloads."
*Bearing:* the target GPU's own rule: pair a pass that leaves warp slots empty (the pyramid chain,
the draws, the culls' serial appends) with one that fills them without competing for the same unit.
The probe rays beside the geometry passes is a good pair; beside the resolve's own rays it is not.

**AMD. "RDNA Performance Guide" (async compute section). GPUOpen, maintained.** [docs]
[still-current]
<https://gpuopen.com/learn/rdna-performance-guide/>

"Async compute fills compute units as graphics waves drain, and should be used to overlap frontend
heavy graphics work. Common overlapping opportunities include Z pre-pass, shadow rendering, and
post-process"; "smaller workgroups (64 threads) usually perform better than larger workgroups when
run async"; and the warning: "async compute performs poorly when executed in parallel with export
bound shaders".
*Bearing:* the other vendor agrees on the pairing and adds two rules Forge can apply blind: 64-wide
workgroups for the passes on the compute queue, and no overlap with export-bound work (the software
rasteriser's merge, the full-screen compose).

**Kostas Anagnostou. "Async compute all the things." Interplay of Light, 27 May 2025.** [web]
[recent]
<https://interplayoflight.wordpress.com/2025/05/27/async-compute-all-the-things/>

A practitioner's survey of what to overlap and why: the compute queue "only has access to units that
involve shader execution (SM/caches) and not geometry processing"; "screen space lighting techniques
like GTAO stress cache and ALU (SM) more, while shadow passes and g-buffer passes put more pressure
on geometry processing and VRAM", so the pairs that work put a screen-space pass on the compute pipe
beside a geometry-bound pass on graphics.
*Bearing:* the pairing table Forge should reproduce in `docs/PROFILE.md` once #95 lands: which pass
ran beside which, and the span. The mover work of §7 adds candidates to the compute side (the mover
TLAS build, the probe wake) that touch neither the geometry units nor the render targets.

---

## Recommendation for Forge

**The movers' range.** Keep the instance table as it is for the statics (Morton-sorted, cells of 64
with `CellBounds`, #38) and append a *movers' range* `[static_count, instance_count)` of at most
`mover_capacity` records (10 000 to start), the same 80-byte `Instance` (`int3 cell`, `float3
local`, a quaternion, a uniform scale, the centre and radius from the same cell, mesh, id,
material). A mover's `id` is its slot in the range, and the record's spare `uint2 pad` gets the
mover index (or `id >= static_count` decides). Movers have cells of 64 of their own whose bounds are
recomputed each frame, so the cell cull keeps working with no special case beyond "these cells'
bounds are fresh".

**The upload, in the graph.** The CPU owns the truth of every mover (the fixed-tick simulation of
§5, in `forge-sim` when it exists, in the demo until then) and writes, every frame, the *whole*
mover array — `{int3 cell, float3 local, float4 rotation, float scale}`, 48 bytes each, 480 KB for
10 000 — into slot `frame % 2` of a host-visible ring (`MoverTransforms[2]`), the transforms
interpolated to the render time on the CPU so the GPU never sees the tick. The first pass of the
frame, `movers/apply` (compute, graphics queue, a thread per mover), writes the instance records of
the range from the current slot and recomputes the movers' cell bounds; it declares the ring slot as
read and the instance buffer and `cells` as written, so the graph orders it before the instance
culls and after the previous frame's readers. It stays on the graphics queue: everything downstream
waits for it and it costs microseconds. Uploading all movers every frame (29 MB/s at 60 fps against
a 64 MB per-frame streaming budget) keeps both slots complete, which the next point needs; a change
list (Unreal's dirty primitives) is the refinement for 100 000 movers.

**Previous transforms.** No second history: the *other* slot of the ring is last frame's mover
array, complete, and `Frame` carries both addresses (`movers`, `movers_prev`). The motion pass
(`temporal/motion`) reads the visibility id; for a static pixel it keeps today's camera-only vector;
for a mover's pixel it reconstructs the object-space point of the triangle (the resolve's vertex
fetch and analytic barycentrics), transforms it by the previous rotation and scale, adds the
previous position relative to *this* frame's camera — `float3(prev.cell − camera_cell) · CELL_SIZE +
(prev.local − camera_local)`, exact by the cells' rule — projects it with the previous camera
expressed from this frame's camera-relative space (`Frame::prev_cull_view` already is), removes the
two jitters, and writes the difference. A 3 × 3 nearest-depth dilation (Pedersen) picks the vector
the TAA history reads. DLSS gets the same buffer. A mover spawned this frame writes a zero object
vector once, and the clamp rejects its history.

**Occlusion: movers take pass 2.** Instance cull 1 skips the movers' cells (listed as deferred
without a test), instance cull 2 tests them against this frame's pyramid, and their clusters go
through cluster cull 2 and mesh pass 2. This is correct by the two-pass property (§1): a mover is
never judged by where it was, and it is drawn in the frame it is visible. It costs 10 000 instance
tests in cull 2 (the whole cull is 0.14 ms for a million today) and the movers' clusters in a pass
whose cull now takes 0.03 ms. What it gives up: movers are absent from the pyramid pass 1 uses next
frame (built after pass 1), so a large ship never occludes the belt behind it in pass 1 — wasted
work, not a wrong image, negligible for cars and fighters. The upgrade for a capital ship is
Nanite's rule, testing the mover in pass 1 with its previous transform against the previous pyramid;
the previous transform is already in the ring, so it is a shader change when a demo shows the need.
Cluster LOD needs nothing: the DAG selects per frame from the current transform, and the error
threshold is what prevents pops when a ship closes at 300 m/s. What to watch is page streaming
(D-025): the demo should count the frames a fast mover draws at a coarser cut than its `page_need`
asks for.

**The TLAS: a second, small structure, rebuilt every frame.** The numbers rule out the obvious
options: rebuilding the city's structure per frame costs what it costs once today, 12 ms for
1 000 001 instances, more than the whole frame, and a refit of it keeps that topology and, by the
vendors' word, is not worth the quality. So the static TLAS is built once as now, and a *mover TLAS*
over the movers' range is built every frame by a graph pass on the compute queue (`rt/movers tlas
[compute]`, after `movers/apply`, overlapping the culls): its 64-byte instance records come from the
existing `tlas_instances_main` over the range (positions in the scene frame from the cells), and its
build uses `PREFER_FAST_BUILD` with no update flag. Every ray query traces two structures: shadow
rays (accept-first-hit, opaque) trace the mover TLAS first — small, mostly empty, a hit ends the ray
— then the statics; mirror, ice and probe rays trace both and keep the nearer `t`. `Frame` gets
`tlas_movers` next to `tlas`. Cost estimate, to be replaced by measurement: the ballad's
3000-instance TLAS took 1 ms in a one-shot submission that includes its overhead, so a 10 000-mover
build inside the graph should land between 0.1 and 0.5 ms, off the critical path; the second
traversal should add 10–30 % to the ray passes (0.14 ms of shadow rays, 0.10 of mirror rays, 0.6 of
probe rays at 1440p). The A/B the owner asked for is one flag: `ALLOW_UPDATE` on the single
structure and an update per frame, measured against the two-structure frame at 1 000, 10 000 and
100 000 movers. The third option, NVIDIA's partitioned TLAS with the movers in its global partition,
is loaded only where `VK_NV_partitioned_acceleration_structure` exists (the cross-vendor rule) and
only after the portable scheme has numbers. BLASes do not move: a ship or a car is rigid; refits
arrive with Phase 3's tumbling rocks.

**Probes (#69): a wake list from the movers' spheres, with hysteresis.** The irradiance needs no
wake — the probe rays trace the mover TLAS and the 97 % blend follows a mover in about a tenth of a
second, as DDGI's authors show with their beach balls. What needs waking is the state D-036 froze: a
probe's place and active flag after eight updates. Rule, run as `gi/probe wake` (compute queue,
before the probe rays, a thread per mover per cascade): for each mover take the swept region between
its previous and current bounding spheres, inflated by one probe spacing of the cascade; every probe
whose cell it covers and whose age is past the settling window gets `age = 0` (it re-runs relocation
and classification for eight updates) *if* the mover entered or left that cell this frame — a mover
that stays over a probe leaves it alone, and the probe re-classifies against the parked car as it
would against a wall. Hysteresis in two forms: a woken probe cannot be woken again for 8 updates (a
cooldown in `probe_data`), so a car idling on a cell border does not restart its probe every frame;
and, after McGuire's rule, woken probes blend at 90 % instead of 97 % during their window, then
return, so the light adapts in a few frames where something just moved. A per-frame cap (512 wakes,
the rest carried in a list) bounds the cost, Lumen-style. Sizes: a 5 m car at the 4 m cascade
touches a few dozen probes; 100 cars in view are a few thousand candidates a frame, most rejected by
the enter/leave test. The stat for the overlay is *probes woken this frame*. Later, RTXGI's
variability per cascade is the "settled" signal that lets #95 skip a cascade's update until a wake
or a light change.

**What #95's double-buffering buys.** Today frame N's probe update waits for frame N−1's resolve
(the atlases are single images), so the probes overlap only the geometry passes. With the two
atlases and the two sky tables double-buffered like the TAA history (+53 MiB for the atlases), the
compute batch of frame N — the probe wake, the probe rays and blend, the sky tables, and now the
mover TLAS build — starts during frame N−1's resolve, TAA and post, which is where NVIDIA's rule
(fill empty warp slots with non-conflicting work) applies best: the post chain is small dispatches
and export-bound passes that leave the RT cores idle. The hazards are the ones the vendors list: the
probe rays beside the resolve's own rays and texture reads contend for the same units and memory,
and AMD warns off overlapping export-bound shaders, so the batch should be measured with and without
each member. Expectation: a gain of the same order as #77's 0.11 ms, not more, and a cleaner
critical path for the mover build and wake.

**Demo plan.** A flag in each demo, off by default (`--movers N`), so the golden captures stay put;
a separate binary only if the flag grows heavy. *Belt:* N ships (100, 1 000, 10 000) on seeded
Catmull-Rom splines threaded through the belt, control points chosen by rejection against the
asteroids' spheres, orientation from a rotation-minimising frame with a roll proportional to the
lateral acceleration, formations as offsets in the leader's frame with Reynolds' separation; one
hull mesh (a metal row with a mirror ray, #80's first step). *City:* N cars on a lane graph derived
from the street grid (one lane a direction to start), IDM car-following against the car ahead and a
virtual stopped car at each intersection's fixed-cycle light, spawned and despawned only in the
grid's edge cells outside the frustum and beyond 1 km, so no car appears on screen. Both run a 60 Hz
fixed tick from `Seed::derive`, `dmath` only, neighbours ordered by index, with a CI digest of every
transform at tick 600 at one and six workers (D-016), and interpolation at render. *Numbers to
report* (`docs/PROFILE.md`, 1600 × 900 and 1440p, before and after, movers off and on): GPU ms per
pass (instance culls 1/2, cell culls, cluster culls 1/2, mesh passes 1/2, the pyramid,
`movers/apply`, `temporal/motion`, the resolve and its ray passes, `gi/*`), the mover TLAS build in
ms against N, the ray passes with one and with two structures, the single-structure refit A/B,
probes woken per frame and the probe passes' ms, host uploads per frame, and the CPU tick.
*Captures:* a ship crossing the frame with TAA on and off and with DLSS, the ghost trail measured in
pixels behind the hull (0 expected with the object vectors; the trail without them is the "before"
image), its shadow and its reflection in the ice, the street under a moving car with the probes'
irradiance shown; and the whole verification batch (`tools/compare.sh`, `--no-occlusion`,
`--no-cone`, the fallback, `--show-culled`) at 0 pixels with the flag off and with `--movers 0`.

---

## What the numbers say

Forge's own: a TLAS over 1 000 001 instances builds in 12 ms and one over 3 000 in 1 ms (one-shot,
D-029), the city's instance culls take 0.14 ms for a million records and cluster cull 2 0.03 ms
after #92, the probes cost 0.77 ms at 1600 × 900 (0.58–0.62 of passes) with 128 rays per probe and a
97 % blend, #77's async move saved 0.11 ms of a 2.36 ms frame while the probe zones stretched 0.60 →
1.35 ms, and the renderer uploads 1.2 KiB a frame against a 64 MB streaming budget, so 480 KB of
mover transforms is noise. The vendors: build the TLAS rather than update it (NVIDIA 2019), its
per-frame rebuild costs in proportion to the instance count (Epic), fewer instances build faster and
tight, unskewed instances trace faster (AMD), BLAS updates are for limited deformation and
compaction saves more than half the memory (NVIDIA, RTXMU), and NVIDIA's partitioned TLAS sample
moves 170 000 objects among 1.2 million statics by rebuilding only the partitions touched. The
temporal side: DLSS wants depth, motion vectors at render resolution, jitter-free matrices and
pixel-space jitter; INSIDE's recipe is 16 Halton samples, nearest-depth velocity and a 3 × 3 clip.
The probes: hysteresis 90–99.5 % viable, 97 % recommended, 95 % with 192–256 rays converges in about
100 ms at 60 Hz, drop it near an object that crosses a cell in a frame; relocation moves a probe at
most 45 % of a cell; RTXGI's variability pauses tracing when settled. Async compute: 5–10 % on AMD
and nothing on NVIDIA for Hitman in 2016; pair passes that stress different units and avoid
export-bound overlap. GPU-driven movers at scale: Siege's thousands of rubble objects at 73 %
culling efficiency.

---

## Checked and left out

Kept so the bibliography is auditable: things looked for and not above, with the reason.

- **An Activision GPU-driven pipeline talk** — none found. Activision Research's publication list
  and the *Advances* course indexes of 2016–2024 give z-binning and clustered culling (Drobot 2017),
  the *Call of Duty: WWII* material work and the Caldera data set; nothing on instance tables or
  occlusion for movers. Siege, Frostbite and Nanite stand for the shipped pipelines.
- **Tiago Sousa, Jean Geffroy, "The Devil is in the Details: idTech 666", SIGGRAPH 2016** — the talk
  and its PDF are listed on the Advances 2016 index, but the only readable statement of its async
  compute use is a wiki's engine summary; not citation grade, so Doom 2016 is not an entry.
- **Geffroy, Wang, Gneiting, "Rendering the Hellscape of Doom Eternal", SIGGRAPH 2020** — confirmed
  from the Advances 2020 index and a course summary (geometry caches, gore, decals, 60 fps); folded
  into the Siege entry as a one-line note, since only its topic list was readable.
- **Johannes Deligiannis, Jan Schmid (DICE), "It Just Works: Ray-Traced Reflections in Battlefield
  V", GDC / GTC 2019** — the talk exists (GTC S91023, 48 minutes, shader generation to denoising),
  but its statements on BVH budgets and dynamic objects could not be read.
- **Thomas & Dunn, "Practical DirectX 12", GDC 2016, and O'Donnell, "FrameGraph", GDC 2017** — both
  confirmed (GDC Vault 1023507 and 1024612; the GPUOpen, NVIDIA and Slideshare listings); the
  first's async-compute slides were not readable and the vendors' current guides replace it, the
  second is named in §6's introduction and carried by `task-system.md`.
- **The DirectX Raytracing specification** (`DirectX-Specs/d3d/Raytracing.md`) — GitHub's page for
  the file is too large to render and the API fetch returned nothing usable, so its wording on
  update degradation is not quoted; the Vulkan chapter carries the rules.
- **Lumen's far field as a second TLAS** — the documentation describes a per-component near/far
  membership with its own trace range; whether that is a separate acceleration structure in the
  engine was not confirmable from the pages read, and the entry says so.
- **Metro Exodus Enhanced Edition's probe updates; FSR's motion-vector requirements** — not searched
  within the budget; DDGI's authors and RTXGI cover the first, and Forge's upscaler is DLSS.
- **Nanite's previous-transform test and velocity write as primary quotes** — the deep-dive PDF is
  on a blocked host; both details come from a university course summary and a rendering blog and are
  marked so in §10.
- **Two-pass occlusion write-ups** (Kruskonja's Medium post, the Bevy meshlet pull request), **Media
  Molecule's Dreams** (named in Aaltonen's post), and **a Ray Tracing Gems II chapter on dynamic
  scenes** — blogs and pull requests are not cited, Evans's 2015 talk (`large-worlds.md`) says
  nothing about occlusion passes, and no such chapter was found by search.
- **Continuum and crowd-scale traffic** (Sewall et al. 2010 and after) — out of scope; IDM and MOBIL
  are what a demo of cars needs, and #90 is the larger issue.
- **Epic's Mass Traffic despawn rules** — the City Sample page describes spawners and the ZoneGraph
  but not how vehicles leave; §7 states Forge's own rule.

---

## Verification notes

Checked on 2026-09-25 with WebSearch and WebFetch only; no browser pane and no YouTube pages. The
session's egress proxy allowed WebFetch to reach `github.com` and refused every other host tried
(advances.realtimerendering.com, jcgt.org, arxiv.org, dev.epicgames.com, developer.nvidia.com,
gpuopen.com, gdcvault.com, red3d.com, gafferongames.com, interplayoflight.wordpress.com,
filmicworlds.com, onlinelibrary.wiley.com, link.aps.org, journals.sagepub.com, dl.acm.org,
docs.vulkan.org, ubisoft.com). A `gh api` call for the DXR specification returned nothing usable.
Verification therefore has two grades.

- **Fetched and read (GitHub):** the Streamline DLSS programming guide (both the NVIDIAGameWorks and
  the NVIDIA-RTX organisations); RTXGI-DDGI's README, `docs/DDGIVolume.md` and `ChangeLog.md`
  (version 1.3 in the README; the changelog carries no dates; variability appears at 1.3.5); RTXMU's
  README; the Vulkan specification's `chapters/accelstructures.adoc` and the
  `VK_NV_partitioned_acceleration_structure` proposal in KhronosGroup/Vulkan-Docs; the
  `vk_partitioned_tlas` README and the `ray_tracing_animation` chapter of
  `vk_raytracing_tutorial_KHR` (nvpro-samples); McGuire, Majercik and Marrs's DDGI articles (parts 3
  and 6, morgan3d/articles; part 6 marks its probe-sleeping section as unwritten); Playdead's
  `temporal` repository listing. Quotes from these are verbatim.
- **Confirmed through the search engine's record of the primary page** (title, authors, venue,
  dates, and the sentences quoted, which are the search engine's extracts of the page named): Haar &
  Aaltonen 2015 and Aaltonen's post; Wihlidal 2016 (GDC Vault 1023109, the archive.org transcript);
  Karis, Stubbe, Wihlidal 2021 and Epic's Nanite page; Epic's Mesh Drawing Pipeline page; El
  Mansouri 2016 (GDC Vault 1023287, the archive.org transcript with the culling table, the
  gamedeveloper.com write-up); Karis 2014 (the Advances 2014 index); Pedersen 2016 (GDC Vault
  1022970, an LTH report's summary of the recipe); Yang, Liu, Salvi 2020 (Wiley, the Eurographics
  library); Burns & Hunt 2013 (Semantic Scholar: JCGT 2(2), 55–69) and Hable 2021; NVIDIA's 2019 and
  2020 best-practice posts; AMD's RDNA Performance Guide and RRA article; Epic's Ray Tracing
  Performance Guide and Lumen pages (with forum threads quoting the far-field settings); NVIDIA's
  Mega Geometry announcement (with the Khronos and Vulkan news mirrors); Boksanský, Wimmer, Bittner
  2019 (TU Wien, SpringerLink: pages 159–182); the AC Shadows talk (the Advances 2025 content
  listing) and Ubisoft's Q&A; Majercik et al. 2019 (NVIDIA Research, the JCGT listing: 8(2), 5 June
  2019) and 2021 (JCGT 10(2), 3 May 2021; authors Majercik, Marrs, Spjut, McGuire — the first search
  wrongly named the resampling paper's authors, corrected here); Wright, Narkowicz, Kelly 2022
  (Epic's SIGGRAPH 2022 page, the Advances 2022 index); Reynolds 1987 (red3d.com, the SIGGRAPH
  history page) and 1999 (red3d.com, with the list of behaviours); Wang et al. 2008 (ACM DL, JKU,
  Microsoft Research); Treiber, Hennecke, Helbing 2000 (APS, arXiv) and Kesting, Treiber, Helbing
  2007 (SAGE, the author copy's listing); Epic's City Sample page; Fiedler's three posts (with the
  GitHub mirror of the site's sources); Meyer 2016 (GDC Vault 1023129, the GDC PDF listing);
  NVIDIA's async-compute post (with its forum mirror); Anagnostou 2025 (with the daily.dev mirror's
  extracts).
- **Weaker confirmations, stated plainly.** Nanite's use of the previous frame's transforms in pass
  1 comes from a University of Illinois course page summarising the talk, and the velocity sentence
  from a rendering blog's write-up; neither is Epic. Unreal's `FPrimitiveSceneData` fields and the
  dirty-primitive gathering come from third-party write-ups and the 4.26 API reference's listing,
  not from the 5.8 page quoted. Wihlidal's "almost for free" is the trade press's paraphrase.
  Hitman's 5–10 % is press coverage of the talk, not the slides. The Haar & Aaltonen talk's content
  beyond its part titles is not re-quoted; `gpu-geometry.md` already marks its cluster and
  reprojection details as from memory. The DLSS guide does not, in the sentences read, distinguish
  object motion from camera motion; the entry says so. The NVIDIA async-compute post's author was
  not confirmed; it is cited as NVIDIA's. Boksanský's chapter is described from the publisher's and
  TU Wien's abstracts. The Lumen far-field sentences are from forum posts quoting the documentation
  and from the documentation's search extract.
- **Forge's own numbers** (the TLAS's 12 ms and 1 ms, the culls' and probes' costs, the #77 and #92
  tables, the uploads per frame, the 80-byte record, the cell size of 1 km) are from
  `docs/DECISIONS.md` (D-004, D-020, D-029, D-036), `docs/PROFILE.md`, `docs/demos/city-blocks.md`,
  `shaders/meshlet.slang` and `crates/forge-world/src/cells.rs` (re-exported by
  `forge_render::cells`) as of 2026-09-25.
- **Numbers to re-check before they enter a spec:** every estimate in the recommendation (the mover
  TLAS's 0.1–0.5 ms, the 10–30 % on the ray passes, the wake counts, the gain from double-buffering)
  is an estimate, marked as such, to be replaced by the demo's timings; the 3000-instance 1 ms
  includes a one-shot submission's overhead; the PTLAS sample's counts are the sample's, with no
  timings published; the DDGI convergence figure (100 ms) is for 192–256 rays at 95 %, not Forge's
  128 at 97 %; Hitman's percentages are 2016 hardware.
