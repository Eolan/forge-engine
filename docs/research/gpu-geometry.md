# GPU-Driven Geometry Pipeline

Forge targets huge procedural open worlds on Vulkan 1.4 (`ash`) with Slang shaders, on hardware ranging from a Blackwell RTX 5070 Ti (mesh shaders, cluster acceleration structures, device-generated commands, 64-bit image atomics) down to GPUs with none of it. The owner's principle, "always use the power of the modern GPU: handle the expansive geometry on the GPU with the appropriate shaders," means a GPU-driven pipeline: the CPU submits a handful of indirect commands and compute, task and mesh shaders decide per frame which instances, clusters and detail levels reach the rasterizer or the ray tracer. This file collects the verified sources behind that design: why the geometry shader died, how to size meshlets, how two-pass hierarchical-Z occlusion culling works, what Nanite and its 2022–2026 descendants (skeletal, tessellation, foliage/voxels, NVIDIA RTX Mega Geometry, AMD work-graph mesh nodes) actually do, which tools build cluster DAGs, and what shipped games prove. It ends with an opinionated build order.

> **State of the art in five sentences.** Production engines no longer draw meshes; they draw *clusters* of 64–128 triangles chosen on the GPU by compute or task shaders doing frustum, backface-cone and two-pass hierarchical-Z occlusion culling, then hand survivors to mesh shaders (or an indirect-count draw on older hardware) that write a visibility buffer instead of a G-buffer. Continuous level of detail is a DAG of cluster groups built offline by graph partitioning plus attribute-aware, border-locked quadric simplification (Nanite 2021, meshoptimizer's `clusterlod.h`, Bevy's meshlet renderer in Rust), with a compute software rasterizer for the sub-pixel triangles hardware handles badly. The same clusters now feed ray tracing through cluster acceleration structures (NVIDIA RTX Mega Geometry, `VK_NV_cluster_acceleration_structure`, 2025) and can be tessellated and displaced per frame on the GPU, which is why hardware tessellation and displacement micromaps are being retired. Vulkan 1.4 makes the enablers core (buffer device address, descriptor indexing, draw-indirect-count, dynamic rendering, synchronization2, push descriptors, streaming-transfer guarantees), and Slang, hosted by Khronos and shipped in the Vulkan SDK, compiles task, mesh, compute and all ray-tracing stages to SPIR-V from one source. Alan Wake 2 (2023), Assassin's Creed Shadows (2025) and Doom: The Dark Ages (2025) ship this kind of pipeline; Rockstar publishes nothing.

---

## 1. Why geometry shaders are obsolete and what replaced them

**Kubisch, C. "Introduction to Turing Mesh Shaders." NVIDIA Technical Blog, 2018.** [web] [foundational]
https://developer.nvidia.com/blog/introduction-turing-mesh-shaders/ — see also Reed, N., "Mesh Shader Possibilities," 2018: https://www.reedbeta.com/blog/mesh-shader-possibilities/
The post (17 Sep 2018) that introduced the task/mesh pipeline as the replacement for vertex, tessellation and geometry shaders: a workgroup cooperatively builds a *meshlet* on chip and hands it to the rasterizer. It fixes the first numbers, up to 64 vertices and 126 primitives ("the '6' in 126 is not a typo": 3·126+4 count bytes fills three 128-byte index blocks) and 32-thread workgroups. Reed's same-month post states the case against geometry shaders: one thread generating geometry alone causes divergence and large I/O, while tessellation suffers coarse factor granularity and uneven vendor performance.
*Bearing:* Origin of the 64/126 convention and of the compute-model-for-geometry idea Forge is built on.

**Kubisch, C. "Mesh Shading for Vulkan." Khronos Blog, 2022; Khronos, "VK_EXT_mesh_shader proposal."** [web] [still-current]
https://www.khronos.org/blog/mesh-shading-for-vulkan — https://github.com/KhronosGroup/Vulkan-Docs/blob/main/proposals/VK_EXT_mesh_shader.adoc
Published 1 Sep 2022 with Vulkan 1.3.226 for DirectX 12 compatibility. Tessellation gives "very limited control over the triangles created," geometry shaders use a single-thread model that runs poorly, API portability is easy but "portability in performance among vendors is much harder," and task shaders "may add overhead" unless they cull or amplify. The proposal lists the properties to read at startup (`maxPreferredTask/MeshWorkGroupInvocations`, `maxMeshOutputVertices/Primitives`, `maxTaskPayloadSize`, compact-output preferences) and states that mesh shaders "are not necessarily a performance win"; their purpose is flexibility.
*Bearing:* Forge must query the preference properties and keep a per-vendor tuning table (32 invocations on the RTX 5070 Ti, 128 on RDNA).

**Kristóf, T. "How mesh shaders are implemented in an AMD driver" and "Mesh shaders arrive on your Linux computers." timur.hu, 2022.** [web] [still-current]
https://timur.hu/blog/2022/how-mesh-shaders-are-implemented — https://timur.hu/blog/2022/mesh-shaders-arrive-on-linux — AMD RDNA Performance Guide: https://gpuopen.com/learn/rdna-performance-guide/
On AMD "each thread (SIMD lane) can only output up to 1 vertex and 1 primitive," so the driver stages outputs in LDS and pads the hardware workgroup to `max(api workgroup, max vertices, max primitives)`; idle lanes still wait at barriers. At launch RADV and Intel's ANV shipped the extension in Mesa 22.3 (RADV experimental behind `RADV_PERFTEST=ext_ms`), NVIDIA in beta drivers, and RDNA1 cannot support it "due to some HW limitations." AMD's RDNA guide adds that "culling primitives using a geometry shader or cull distances is expensive."
*Bearing:* The fallback is real: RX 5000 and pre-Turing GPUs have no mesh shaders; on AMD the mesh workgroup size should equal the meshlet's vertex/primitive maximum.

## 2. Mesh shader best practices and limits

**Kubisch, C. "Using Mesh Shaders for Professional Graphics." NVIDIA Technical Blog, 2020.** [web] [still-current]
https://developer.nvidia.com/blog/using-mesh-shaders-for-professional-graphics/
Measured on Turing and Ampere (8 Dec 2020): 64 vertices / 84 primitives and 32-thread workgroups were the sweet spot; the 27M-triangle Lucy scan took 0.9 ms with mesh shaders versus 1.8 ms for the vertex pipeline with 32-bit indices (0.8 vs 1.1 ms with 16-bit). Task-shader cluster culling with bounding shapes and a normal cone "proved most beneficial," and per-primitive culling gave near 2x in visibility-buffer/depth-prepass scenes with sub-pixel triangles (Rungholt, 0.26 px² average).
*Bearing:* Forge's regime (dense geometry, depth prepass into a visibility buffer) is where the measured 2x comes from; the RTX 3080 test machine is the Ampere case here.

**Mihut, A., Kubisch, C., Kraemer, M. "Advanced API Performance: Mesh Shaders." NVIDIA Technical Blog, 2021.** [web] [still-current]
https://developer.nvidia.com/blog/advanced-api-performance-mesh-shaders/
NVIDIA's checklist (25 Oct 2021): task payload under 108 bytes if possible, never over 236; 64 vertices / 126 triangles with sweet spots at 40 and 84; LOD selection in the task stage, primitive culling in the mesh stage; bake meshlets offline; separate code for dense and sparse topologies; never emulate fixed function. In Vulkan the pre-allocated mesh outputs are readable and writable and can double as scratch memory.
*Bearing:* Fixes the task-payload budget (cluster range plus instance id and LOD bits in 108 bytes) and confirms offline meshlet baking in the cooker.

**Oberberger, M., Kuth, B., Meyer, Q. "Mesh shaders on AMD RDNA graphics cards" (GPUOpen series); Kramer, L., Oberberger, M. "Mesh Shaders in AMD RDNA 3 Architecture." GDC 2024.** [web] [still-current]
https://gpuopen.com/learn/mesh_shaders/mesh_shaders-optimization_and_best_practices/ — https://gpuopen.com/learn/mesh_shaders/mesh_shaders-index/ — https://gpuopen.com/events/amd-at-gdc-2024/
The five-part series (Dec 2023; best practices dated 16 Jan 2024) recommends the opposite corner: 128 vertices and 256 triangles with 128-thread workgroups mapped 1:1 (thread *i* writes vertex *i*, primitive *i*, then *i*+128), wave64, output arrays sized to real limits, and amplification shaders that process 32–64 elements and launch at least 32 mesh workgroups with payloads under 16 KB. RDNA 2 exports one vertex/primitive per thread; RDNA 3 adds wave-wide export offsets.
*Bearing:* A 128-triangle cluster is the largest unit inside both vendors' comfort zones (4 primitives per lane at 32 invocations on NVIDIA, 1 per lane at 128 on AMD); do not go to 256.

**Kuth, B., Oberberger, M., Kawala, F., Reitter, S., Michel, S., Chajdas, M., Meyer, Q. "Towards Practical Meshlet Compression." arXiv 2404.06359, 2024.** [paper] [recent]
https://arxiv.org/abs/2404.06359
A codec for decompression inside a mesh shader: triangles ordered into optimal generalized strips by mixed-integer linear programming, 16:1 index compression versus the vertex pipeline, crack-free attribute quantization, 15.5 M triangles decoded and rendered in 0.59 ms on an RX 7900 XTX (v2, Aug 2024). The authors note the kinship with Nanite's strip decoding.
*Bearing:* Design the cluster format for in-shader decode (strips plus positions quantized to cluster bounds); streaming a planet is bounded by geometry bytes.

## 3. GPU culling: clusters, hierarchical Z, indirect draws

**Haar, U., Aaltonen, S. "GPU-Driven Rendering Pipelines." SIGGRAPH 2015, Advances in Real-Time Rendering in Games.** [talk] [foundational]
https://advances.realtimerendering.com/s2015/ — https://advances.realtimerendering.com/s2015/aaltonenhaar_siggraph2015_combined_final_footer_220dpi.pdf
The course page confirms both halves: Assassin's Creed Unity's per-material instance batching with mesh clustering, and RedLynx's clean-slate design using async compute and indirect dispatch to render "hundreds of thousands of independent objects with unique meshes, textures and decals at 60 FPS" on consoles. From the slides (PDF fetched; details from memory, see verification notes): fixed-size clusters with bounds and backface cones, occlusion against a depth pyramid built from previous-frame depth reprojected to the current camera, triangle-level culling in compute, compacted multi-draw-indirect buffers.
*Bearing:* The canonical cluster-culling-plus-HZB description; every later system reuses previous-frame depth as the first occluder.

**Wihlidal, G. "Optimizing the Graphics Pipeline with Compute." GDC 2016 (Frostbite).** [talk] [foundational]
https://www.wihlidal.com/projects/fb-gdc16/ — https://www.gdcvault.com/play/1023109/Optimizing-the-Graphics-Pipeline-With
Compute pushes triangle throughput past fixed-function limits with "per-primitive filtering kernels" and just-in-time geometry optimization, integrated into Frostbite for most titles. The deck holds the per-triangle filters (backface, small-primitive, frustum, depth; from memory) and async-compute scheduling later pipelines copy.
*Bearing:* Compute plus `vkCmdDrawIndexedIndirectCount` is Forge's no-mesh-shader fallback; these filters are what it must implement.

**Geffroy, J., Gneiting, A., Wang, Y. "Rendering the Hellscape of Doom Eternal." SIGGRAPH 2020, Advances in Real-Time Rendering in Games (id Software).** [talk] [still-current]
https://advances.realtimerendering.com/s2020/index.html — https://advances.realtimerendering.com/s2020/RenderingDoomEternal.pdf
id Tech 7 at a locked 60 FPS on all platforms: geometry caches, gore, decals, material compositing, water, and the binning/culling work that moved culling to the GPU to cut CPU load; the Vulkan-native predecessor of id Tech 8.
*Bearing:* The most detailed public account of a shipping Vulkan-only, GPU-culled engine; its material compositing pairs well with a visibility buffer.

**Drobot, M. "Geometry Rendering Pipeline Architecture." Activision Research (REAC), 2021.** [talk] [still-current]
https://research.activision.com/publications/2021/09/geometry-rendering-pipeline-architecture — https://research.activision.com/publications
Call of Duty's pipeline (17 Sep 2021) unifies terrain, high-density meshes, procedural meshes and foliage "via merging, into highly optimized intermediate formats" for a Forward+ renderer with visibility-buffer-style rendering and software variable-resolution shading, with performance analysis of indoor, outdoor and heavy-overdraw scenes. The same list holds Drobot's 2017 "Rendering of Call of Duty: Infinite Warfare" (Digital Dragons) and Vance's 2021 "Rendering Engine Architecture at Activision."
*Bearing:* Precedent for one merged cluster stream for terrain, procedural meshes and foliage rather than separate renderers.

## 4. Virtual geometry: ancestors, Nanite, and the 2022–2026 follow-ups

**Hoppe, H. "Progressive Meshes." SIGGRAPH 1996.** [paper] [foundational]
https://hhoppe.com/proj/pm/ — https://www.cs.princeton.edu/courses/archive/fall03/cs526/papers/hoppe96.pdf
A lossless continuous-resolution representation: base mesh plus vertex splits, supporting geomorphs, progressive transmission and selective refinement while preserving materials, normals and texture coordinates (pp. 99–108).
*Bearing:* Cluster DAGs are progressive meshes with the refinement unit coarsened to 128 triangles so the GPU can pick levels in parallel.

**Garland, M., Heckbert, P. S. "Surface Simplification Using Quadric Error Metrics." SIGGRAPH 1997.** [paper] [foundational]
https://history.siggraph.org/learning/surface-simplification-using-quadric-error-metrics-by-garland-and-heckbert/ — https://graphics.stanford.edu/courses/cs348a-04-winter/Papers/garland-heckbert.pdf — DOI 10.1145/258734.258849
Iterative vertex-pair contraction ordered by an accumulated plane-distance quadric; joins disconnected regions and tolerates non-manifold input. It is the simplifier inside meshoptimizer and every DAG builder below.
*Bearing:* The per-group error stored in the DAG must be a monotonic function of this error or GPU LOD selection produces cracks.

**Cignoni, P., Ganovelli, F., Gobbetti, E., Marton, F., Ponchio, F., Scopigno, R. "Batched Multi Triangulation." IEEE Visualization 2005.** [paper] [foundational]
http://vcg.isti.cnr.it/publication/2005/CGGMPS05/ — https://vcgdata.isti.cnr.it/Publications/2005/CGGMPS05/BatchedMT_Vis05.pdf — code: https://github.com/cnr-isti-vclab/nexus
The direct ancestor of Nanite's structure: mesh fragments at different resolutions arranged in a DAG encoding dependencies, so any cut through the DAG is a crack-free mesh, with fragments sized for batched GPU rendering (pp. 207–214); Nexus is its living implementation.
*Bearing:* States the DAG-cut invariant: draw a cluster iff its own error passes and its parent group's does not.

**Burns, C. A., Hunt, W. A. "The Visibility Buffer: A Cache-Friendly Approach to Deferred Shading." JCGT 2(2), 2013.** [paper] [foundational]
https://jcgt.org/published/0002/02/04/ — https://jcgt.org/published/0002/02/04/paper.pdf
Replace the G-buffer with a per-sample triangle index and instance id (as few as four bytes) and fetch attributes in the shading pass; storage and bandwidth drop sharply and shading decouples from rasterization (pp. 55–69).
*Bearing:* Forge's three rasterizers (mesh, software, fallback) write one 64-bit visibility sample, which keeps three back ends cheap.

**Schied, C., Dachsbacher, C. "Deferred Attribute Interpolation for Memory-Efficient Deferred Shading." HPG 2015.** [paper] [foundational]
https://diglib.eg.org/items/f74abc84-78bd-4be9-a300-61820d92c204 — DOI 10.1145/2790060.2790066
References into a dynamically filled buffer of visible triangles, each stored as a sample point plus screen-space partial derivatives so attributes and their derivatives are reconstructed at shading time; a per-pixel linked list shrinks the buffer further.
*Bearing:* The recipe for analytic derivatives in compute-shaded visibility buffers; `VK_KHR_compute_shader_derivatives` on the 5070 Ti is the hardware shortcut.

**Hable, J. "Visibility Buffer Rendering with Material Graphs." Filmic Worlds, 2021.** [web] [still-current]
https://filmicworlds.com/blog/visibility-buffer-rendering-with-material-graphs/
On an RTX 3070 at 1080p (5 Jul 2021) visibility rendering loses slightly on large triangles (3.01 vs 2.38 ms forward) but wins 23% at 8–10 px and 32.5% at 1 px triangles (4.34 vs 6.43 ms deferred), because 2x2 helper lanes waste up to 4x work; the contribution is chain-rule derivative code generated from the material graph. His 2024 VRS-with-visibility-buffer talk is in the SIGGRAPH 2024 Advances course (https://advances.realtimerendering.com/s2024/index.html).
*Bearing:* Quantifies why Forge's materials should be a graph that emits Slang with analytic derivatives.

**Karis, B., Stubbe, R., Wihlidal, G. "A Deep Dive into Nanite Virtualized Geometry." SIGGRAPH 2021, Advances in Real-Time Rendering in Games (Epic Games).** [talk] [foundational]
https://advances.realtimerendering.com/s2021/ — slides: https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf — capture: López, E., "A Macro View of Nanite," 2021, https://www.elopezr.com/a-macro-view-of-nanite/
The full pipeline "from mesh import all the way to final rendered pixels": 128-triangle clusters, groups partitioned by graph cut and simplified together with locked borders so the DAG is crack-free at any cut, instance culling then persistent-thread cluster culling with two-pass HZB occlusion, a compute software rasterizer using 64-bit atomics for small clusters and hardware for large ones, a visibility buffer, material classification into tiles, page-based streaming. López's frame capture confirms the runtime shape: 384-vertex/128-triangle clusters, `R32G32_UINT` visibility with 25-bit cluster and 7-bit triangle ids plus 32-bit depth, HZB from the previous frame's visible set, 128-thread software-raster groups, per-material passes gated by a material-range texture.
*Bearing:* The reference architecture; Forge's 128-triangle clusters, 25+7-bit ids, two-pass HZB and SW/HW split come from here.

**Epic Games. "Nanite Virtualized Geometry," "Unreal Engine 5.4 Release Notes," "Nanite Foliage." Epic Developer Community documentation, 2024–2026.** [web] [recent]
https://dev.epicgames.com/documentation/unreal-engine/nanite-virtualized-geometry-in-unreal-engine — https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-5.4-release-notes?application_version=5.4 — https://dev.epicgames.com/documentation/unreal-engine/nanite-foliage — 5.5 (secondary): https://alternativeto.net/news/2024/10/epic-games-unveils-unreal-engine-5-5-with-megalights-nanite-upgrades-and-lower-royalties
UE 5.4 (Apr 2024) added experimental "dynamic programmable displacement" that tessellates at runtime "only as much triangle detail as required for the current pixel density," plus a project to move Nanite materials to compute shading. UE 5.5 (Oct 2024) extended Nanite to skeletal meshes (community reports: no morph targets, cloth or grooms at first). UE 5.7 (Nov 2025) introduced Nanite Foliage, still experimental in the 5.8 docs: *Assemblies* micro-instance repeated parts (up to 65k instances each), *Voxels* replace far-field clusters with pixel-sized aggregates that keep animation and material properties, *Skinning* animates wind through bone hierarchies; alpha masking and WPO are incompatible, scenes are capped at 16 M instances, DirectX 12 with SM6 is required, and ray tracing uses fallback meshes by default.
*Bearing:* Epic's roadmap shows what a cluster DAG cannot do natively (aggregates, skinning, displacement) and how each was patched; Forge should plan assemblies and a voxel far-field from the start.

**Kubisch, C., Knowles, P., Gautron, P., Bickford, N., Marvie, J.-E. "NVIDIA RTX Mega Geometry Now Available with New Vulkan Samples." NVIDIA Technical Blog, 2025; Khronos, "VK_NV_cluster_acceleration_structure."** [web] [recent]
https://developer.nvidia.com/blog/nvidia-rtx-mega-geometry-now-available-with-new-vulkan-samples/ — https://docs.vulkan.org/refpages/latest/refpages/source/VK_NV_cluster_acceleration_structure.html — SDK: https://github.com/NVIDIA-RTX/RTXMG
Announced 6 Feb 2025 in RTX Kit: `VK_NV_cluster_acceleration_structure` (extension 570) builds cluster-level acceleration structures (CLAS) and assembles BLASes from CLAS lists with device-side multi-indirect inputs, so animated or LOD-selected clusters get an acceleration structure per frame without full rebuilds; `VK_NV_partitioned_acceleration_structure` rebuilds only changed TLAS partitions. Driver 572.16 (30 Jan 2025) or newer is required; the samples fall back to rasterization otherwise.
*Bearing:* One cluster format for mesh-shader raster, software raster and CLAS ray tracing on both dev GPUs, with a coarse-LOD BLAS fallback elsewhere.

**Chajdas, M. "GDC 2024: Work graphs and draw calls – a match made in heaven!"; AMD, "GPU Work Graphs mesh nodes in Vulkan." GPUOpen, 2024.** [talk] [recent]
https://gpuopen.com/learn/gdc-2024-workgraphs-drawcalls/ — https://gpuopen.com/learn/gpu-workgraphs-mesh-nodes-vulkan/ — https://gpuopen.com/learn/work_graphs_mesh_nodes/work_graphs_mesh_nodes-intro/
Mesh nodes are work-graph leaf nodes that launch a mesh-shader pipeline instead of a compute shader, with PSO switching inside the graph, so draws become part of a GPU-scheduled graph with no CPU barriers; AMD measured ExecuteIndirect 1.64x slower on an RX 7900 XTX (Mar 2024). In Vulkan this is the experimental `VK_AMDX_shader_enqueue` (Oct 2024, beta driver, one node per SPIR-V module) with a stated intent to standardize as EXT/KHR; a SIGGRAPH 2025 course continues the work.
*Bearing:* Not portable today (AMD-only, experimental), but the design target: write culling, LOD and draw stages as data-flow nodes so a work-graph back end can replace the indirect chain later.

**Fang, Y., Wang, Q., Wang, W. "Aokana: A GPU-Driven Voxel Rendering Framework for Open World Games." Proc. ACM CGIT, 2025.** [paper] [recent]
https://arxiv.org/abs/2505.02017 — DOI 10.1145/3728299
A sparse-voxel DAG with LOD and streaming handling "tens of billions of voxels," up to 9x lower memory and 4.8x faster than prior voxel systems, designed to coexist with mesh pipelines. See also Yang et al., "Virtualized 3D Gaussians" (arXiv 2505.06523, 2025), Nanite-style cluster LOD for billions of splats.
*Bearing:* The cluster-DAG pattern generalizes to non-triangle far-field representations, which is how Forge should render distant procedural terrain and foliage aggregates.

**Schütz, M., Lipp, L., Kristmann, E., Wimmer, M. "CuRast: Cuda-Based Software Rasterization for Billions of Triangles." arXiv 2604.21749, 2026.** [paper] [recent]
https://arxiv.org/abs/2604.21749 — code: https://github.com/m-schuetz/CuRast
A three-stage software rasterizer (small triangles go straight to 64-bit atomic depth/id writes, larger ones are binned) renders hundreds of millions of triangles 2–5x (unique) to 12x (instanced) faster than the Vulkan hardware path, while hardware remains "substantially faster" for low-polygon meshes and thousands of separate objects are handled poorly (Apr 2026).
*Bearing:* Independent 2026 confirmation of Nanite's per-cluster split between software and hardware rasterization.

## 5. Tools to build meshlets and LOD DAGs

**Kapoulkine, A. "meshoptimizer" and "clusterlod.h." GitHub, 2016–2026.** [code] [still-current]
https://github.com/zeux/meshoptimizer — https://github.com/zeux/meshoptimizer/blob/master/demo/clusterlod.h — https://github.com/zeux/meshoptimizer/discussions/750
MIT, v1.2 (30 Jun 2026). Clusterization: `meshopt_buildMeshlets`, `buildMeshletsFlex`, `buildMeshletsSpatial` (SAH-optimized, v0.24, the successor of NVIDIA's cluster builder), `computeMeshletBounds` (spheres and cones), `partitionClusters` (v0.23, improved for disconnected clusters in v1.0). Simplification: `simplifyWithAttributes` with per-vertex `Lock`/`Protect` flags, `SimplifyLockBorder`, `SimplifySparse`, `SimplifyPermissive`, `simplifyScale`. `clusterlod.h` (v1.0, 8 Dec 2025) "implements continuous level of detail by generating a hierarchy of clusters that are progressively grouped and simplified, similarly to Nanite" (`clodBuild`, `clodDefaultConfig[RT]`, and since v1.2 `clodBuildHierarchy` for the DAG BVH), depending only on meshoptimizer. Discussion #750 (Aug 2024, with Bevy's author) records that 256v/128t clusters simplify better than 64v/64t because locked border vertices are 30–50% of a small cluster.
*Bearing:* Forge's DAG builder: bind `clusterlod.h` through a thin FFI (or port it); NVIDIA's own LOD sample now uses it.

**Wihlidal, G. "meshopt" crate. crates.io, 2018–2026.** [code] [still-current]
https://crates.io/crates/meshopt — https://docs.rs/meshopt/latest/meshopt/ — https://github.com/gwihlidal/meshopt-rs
v0.6.2 (7 Aug 2026), MIT/Apache-2.0: FFI plus idiomatic wrappers including `build_meshlets[_flex|_spatial]`, `compute_meshlet_bounds`, `compute_cluster_bounds`, `partition_clusters`, `simplify_with_attributes_and_locks`, `simplify_sloppy_with_locks`, `simplify_scale`; it does not wrap `clusterlod.h`.
*Bearing:* Enough to write the DAG builder in Rust on the primitives if the FFI is unwanted; in any case the cache/overdraw optimizer for every cooked mesh.

**Karypis, G., Kumar, V. "METIS – Serial Graph Partitioning and Fill-reducing Matrix Ordering." Karypis Lab, GitHub.** [code] [foundational]
https://github.com/KarypisLab/METIS — paper: https://www.cs.utexas.edu/~pingali/CS395T/2009fa/papers/metis.pdf
Apache-2.0 multilevel recursive-bisection and k-way partitioning (coarsen, partition, project back with refinement) in O(|E|); needs GKlib. Nanite's builder and Bevy use it to cut the cluster-adjacency graph into groups with few shared edges.
*Bearing:* Optional offline dependency; try `meshopt_partitionClusters` first and add METIS only if DAG metrics (root reached, cluster fill) demand it, as Bevy found.

**NVIDIA nvpro-samples. "nv_cluster_builder," "nv_cluster_lod_builder," "vk_lod_clusters," "vk_animated_clusters," "vk_tessellated_clusters." GitHub, 2025–2026.** [code] [recent]
https://github.com/nvpro-samples/nv_cluster_builder — https://github.com/nvpro-samples/nv_cluster_lod_builder — https://github.com/nvpro-samples/vk_lod_clusters — https://github.com/nvpro-samples/vk_animated_clusters — https://github.com/nvpro-samples/vk_tessellated_clusters
Both libraries are archived: `nv_cluster_builder` (archived 13 Nov 2025, concepts "adopted and further optimized in meshoptimizer's `meshopt_buildMeshletsSpatial()`") and `nv_cluster_lod_builder` (cluster, group across previous boundaries, halve with meshoptimizer while locking group borders; archived, pointing to `clusterlod.h`). The samples remain the reference runtime: `vk_lod_clusters` rasterizes clusters with mesh shaders or ray traces them via CLAS, with budgeted RAM-to-VRAM streaming and GPU-driven CLAS allocation in sparse buffers (SDK 1.4.341); `vk_animated_clusters` uses 64-triangle/64-vertex clusters with templates and reports 6.61 to 2.32 ms and 564 to 22 MB of BLAS memory for 8.43 M animated triangles on an RTX 6000 Ada; `vk_tessellated_clusters` subdivides visible clusters per frame with a tessellation table (up to 11 edge segments, recursive beyond) for both raster and CLAS ray tracing.
*Bearing:* These three samples are the specification of Forge's runtime; the archived libraries confirm meshoptimizer as the single build-time dependency.

**JMS55. "Virtual Geometry in Bevy 0.14 / 0.15 / 0.16." jms55.github.io, 2024–2025; Bevy `meshlet` feature.** [web] [recent]
https://jms55.github.io/posts/2024-06-09-virtual-geometry-bevy-0-14/ — https://jms55.github.io/posts/2024-11-14-virtual-geometry-bevy-0-15/ — https://jms55.github.io/posts/2025-03-27-virtual-geometry-bevy-0-16/ — https://docs.rs/bevy/latest/bevy/pbr/experimental/meshlet/index.html
The closest Rust precedent, on wgpu. 0.14 (Jun 2024): meshoptimizer meshlets, a DAG built by grouping ~4 meshlets, simplifying to ~50% with QEM and re-splitting, error as bounding-sphere radii (`self_lod`/`parent_lod`), two-pass occlusion (last frame's visible clusters first, the rest against the new depth pyramid), a packed cluster+triangle visibility buffer; hardware raster only, opaque only, no animation, streaming or compression. 0.15 (Nov 2024): compute software rasterizer with 64-bit atomics (wgpu 22) for clusters under 64 px, about 5x faster overall (4.97 to 0.93 ms on a 3375-bunny scene), meshlets grown to 255v/128t, METIS-seeded builds with group-border-only locks, cluster ids capped near 33 M by 25-bit packing. 0.16 (Mar 2025): METIS-based meshlet generation (UFactor 1, 126-triangle target) for a much better DAG; BVH-based culling named the biggest remaining bottleneck before the author burned out. Bevy 0.19.1 (Aug 2026) still ships the module behind `meshlet` with no later design posts.
*Bearing:* Borrow the DAG/culling structure and the lessons (big clusters, group-border locks, METIS seeding); add what Bevy lacks: disk streaming, skinning, cluster compression, BVH cluster culling and a mesh-shader path (wgpu had none).

## 6. Massive instancing and procedural amplification on the GPU

**van Muijden, J. "GPU-Based Run-Time Procedural Placement in 'Horizon: Zero Dawn'." GDC 2017 (Guerrilla Games).** [talk] [foundational]
https://gdcvault.com/play/1024700/GPU-Based-Run-Time-Procedural — https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn
Compute shaders assemble "fully-fledged environments while the player walks through them, complete with sounds, effects, wildlife and game-play elements" from artist-authored placement graphs, so "painting in a tree line and redirecting roads" is as cheap as moving mountains.
*Bearing:* Placement is a GPU job that emits instance records straight into the culling pipeline's instance buffer, never a CPU scene graph.

**Wohllaib, E. "Procedural Grass in 'Ghost of Tsushima'." GDC 2021, Advanced Graphics Summit (Sucker Punch).** [talk] [still-current]
https://gdcvault.com/play/1027033/Advanced-Graphics-Summit-Procedural-Grass
Every blade is generated on the GPU with its own procedural shape and animation ("acres of grass within reasonable memory and performance limits"), a compute thread per blade driving an indirect draw, then made to read as a natural field.
*Bearing:* Amplification (task shader or compute) is the home for grass, needles and clutter, feeding the same visibility buffer as clusters; never bake blades into meshes.

**Garcia, R. "Waiter, there's an IES in my DGC!" rg3.name (Igalia), 2024; Khronos, "Vulkan Latest Release Includes Device-Generated Commands Extension."** [web] [recent]
https://rg3.name/202409270942.html — https://www.khronos.org/news/permalink/vulkan-latest-release-enables-device-generated-commands
`VK_EXT_device_generated_commands` (Vulkan 1.3.296, 27 Sep 2024) lets the GPU write command sequences (push constants, draws, dispatches, mesh-task launches, ray-tracing dispatches) and switch shaders through Indirect Execution Sets, executed "without any data going through the CPU"; it is Vulkan's ExecuteIndirect, generalizes NVIDIA's NV extension, and was built with Valve, Intel and NVIDIA engineers. That NV lineage plus RTX Mega Geometry is what NVIDIA actually offers in the instancing space; there is no NVIDIA "GPU Instancer" product.
*Bearing:* Needed only when the GPU must change pipelines per draw; with a visibility buffer and one cluster pipeline, Forge can defer DGC and rely on `vkCmdDrawMeshTasksIndirectCountEXT`.

**GurBu Technologies. "GPU Instancer Pro." Unity Asset Store, 2024–2026.** [web] [recent]
https://assetstore.unity.com/packages/tools/utilities/gpu-instancer-pro-290293 — https://gurbu.com/
A Unity extension (v0.12.19, 7 Sep 2026, Unity 2022.3.32+) wrapping `RenderMeshIndirect` and compute shaders for indirect instancing with occlusion culling of vegetation and prefabs, so users need not "master Compute Shaders and GPU infrastructure." Publisher: GurBu Technologies; unrelated to NVIDIA.
*Bearing:* Nothing to adopt; listed only to settle the naming confusion.

## 7. Tessellation and displacement on modern pipelines

**NVIDIA GameWorks. "Displacement-MicroMap-SDK" (archived) and nvpro-samples "vk_displacement_micromaps."** [code] [recent]
https://github.com/NVIDIAGameWorks/Displacement-MicroMap-SDK — https://github.com/nvpro-samples/vk_displacement_micromaps — https://github.com/NVIDIAGameWorks/Displacement-MicroMap-Toolkit
Displacement micro-meshes (Ada, 2022) compressed subdivision-plus-heightfield detail for ray tracing and rasterization. The SDK now states the project "has been archived," that "the vulkan extension `VK_NV_displacement_micromap` is no longer available," and recommends RTX Mega Geometry and `vk_tessellated_clusters` instead; `VK_EXT_opacity_micromap`, which the 5070 Ti exposes, is unaffected.
*Bearing:* Do not build on displacement micromaps; displacement is cluster tessellation on the GPU plus per-frame CLAS builds, with opacity micromaps kept for foliage alpha.

**Unterguggenberger, J. "Mesh Shaders as Replacement for Hardware Tessellation?" johannesugb.github.io, 2026.** [web] [recent]
https://johannesugb.github.io/gpu-programming/mesh-shaders-for-tessellation/
A 23 Mar 2026 study: a terrain sample runs 144 FPS with hardware tessellation versus 119 FPS ported to mesh shaders (21% slower), and a parametric-surface port lost 76%, because a task workgroup passes one payload, one thread sets the mesh dispatch count, and small workgroups under-occupy the GPU; mesh shaders are "not a universal replacement for hardware tessellation."
*Bearing:* Hardware tessellation is fading because cluster subdivision and CLAS displacement cover its uses, not because mesh shaders beat it per triangle; implement subdivision as cluster splitting in compute, not as a mesh-shader tessellator.

**Benyoub, A., Dupuy, J. "Concurrent Binary Trees for Large-Scale Game Components." HPG 2024 (Intel).** [paper] [recent]
https://ggx-research.github.io/publication/2024/07/27/publication-cbt.html — https://arxiv.org/abs/2407.02215 — code: https://github.com/AnisB/large_cbt — 2020 CBT paper DOI 10.1145/3406186
Extends Dupuy's 2020 concurrent binary tree (GPU-parallel longest-edge-bisection tessellation for terrains and planets) to arbitrary polygon meshes via halfedge operators and uses CBTs as memory-pool managers to remove depth limits, rendering planetary-scale geometry in under 0.2 ms on console hardware; also presented in the SIGGRAPH 2024 Advances course.
*Bearing:* The structure for Forge's planet terrain: adaptive bisection from compute with no tessellation stage, emitting patches into the same cluster stream.

## 8. Bindless, descriptors and the Vulkan 1.3/1.4 feature set

**Arntzen, H.-K. "VK_EXT_descriptor_buffer." Khronos Blog, 2022.** [web] [still-current]
https://www.khronos.org/blog/vk-ext-descriptor-buffer
Descriptor sets become `VkBuffer` memory you `memcpy` descriptors into (`vkGetDescriptorEXT`, `vkCmdBindDescriptorBuffersEXT`, `vkCmdSetDescriptorBufferOffsetsEXT`), removing pool management and update overhead for million-object engines (21 Nov 2022). Caveats: some implementations limit descriptor buffers to 32-bit address spaces, push descriptors need special handling, buffers and classic sets cannot mix in one pipeline, and the author calls it a "foot-gun" best suited to API layering, GPU-timeline descriptor manipulation and fully bindless designs with strong validation.
*Bearing:* Start on core descriptor indexing (1.2) plus buffer device address; descriptor buffers are an optional back end, not the foundation.

**Hector, T. (AMD) et al. "VK_EXT_descriptor_heap" proposal and reference page. Khronos, 2024–2026.** [web] [recent]
https://docs.vulkan.org/features/latest/features/proposals/VK_EXT_descriptor_heap.html — https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_descriptor_heap.html
A ratified EXT replacing descriptor sets, layouts and push constants with exactly one sampler heap and one resource heap written through `vkWriteSamplerDescriptorsEXT`/`vkWriteResourceDescriptorsEXT` and indexed directly from shaders, plus a "push data" interface of at least 256 bytes; it targets descriptor-buffer limitations (no image-view objects, separate heaps for cache efficiency, performance portability across direct/heap/buffer hardware), mirrors D3D12 descriptor heaps, and offers `DescriptorSet`/`Binding` decorations for source compatibility.
*Bearing:* The likely end state of Vulkan bindless; make every resource handle "index into a heap" so the switch is a back-end change.

**Khronos Group. "Vulkan 1.4" press release, 2024; Vulkan specification, "Core Revisions" appendix; "You Can Use Vulkan Without Pipelines Today," 2023; Jones, J., "Vulkan Timeline Semaphores," 2020.** [web] [recent]
https://www.khronos.org/news/press/khronos-streamlines-development-and-deployment-of-gpu-accelerated-applications-with-vulkan-1.4 — https://docs.vulkan.org/spec/latest/appendices/versions.html — https://www.khronos.org/blog/you-can-use-vulkan-without-pipelines-today — https://www.khronos.org/blog/vulkan-timeline-semaphores
The appendix fixes what Forge may assume from core: 1.2 promoted buffer device address, descriptor indexing, timeline semaphores, draw-indirect-count and 64-bit buffer atomics; 1.3 dynamic rendering and synchronization2; 1.4 (3 Dec 2024) push descriptors, maintenance5/6, host image copy, index type uint8 and more, plus "streaming transfers" guarantees so applications "stream large quantities of data to a device while simultaneously rendering at full performance"; production 1.4 drivers exist from AMD, Arm, Intel, Mesa, Nintendo, NVIDIA, Qualcomm and others (Khronos, Aug 2025). `VK_EXT_shader_object` (Mar 2023) offers per-stage `VkShaderEXT` objects usable alongside pipelines, with an SDK emulation layer. Timeline semaphores (core 1.2) are the one 64-bit monotonic primitive for GPU-GPU and GPU-CPU ordering.
*Bearing:* Forge's minimum is Vulkan 1.4 with `VK_EXT_mesh_shader` optional: everything the culling and streaming pipeline needs is core.

## 9. Slang as the shading language, and the alternatives

**Khronos Group. "Khronos Group Launches Slang Initiative, Hosting Open Source Compiler Contributed by NVIDIA." Press release, 2024; Slang User's Guide (targets, SPIR-V specifics, automatic differentiation).** [web] [recent]
https://www.khronos.org/news/press/khronos-group-launches-slang-initiative-hosting-open-source-compiler-contributed-by-nvidia — http://shader-slang.org/slang/user-guide/targets — https://github.com/shader-slang/slang/blob/master/docs/user-guide/a2-01-spirv-target-specific.md — http://shader-slang.org/slang/user-guide/autodiff — https://github.com/shader-slang/slang/issues/9444
Since 21 Nov 2024 Slang is an Apache-2.0 project under a multi-company Khronos working group, shipped "as one of the shading language options in the Vulkan SDK," with modules, interfaces and generics, forward/backward autodiff (`fwd_diff`/`bwd_diff`, `[Differentiable]`, `DifferentialPair<T>`) and SPIR-V, HLSL, GLSL, WGSL, Metal and CUDA targets; Valve shipped Slang-generated SPIR-V in Counter-Strike 2 and Dota 2. The targets table lists mesh and all six ray-tracing stages for Vulkan, the SPIR-V page lists "mesh shader" and "ray tracing, inline ray tracing" as supported, and issue #9444 (Dec 2025, fixed) shows `[shader("mesh")]` with `[outputtopology]` compiling to SPIR-V; the D3D12 row of the targets table carries a stale "does not currently support mesh shaders" note.
*Bearing:* Slang covers every stage Forge needs from one module system; autodiff is a free option for neural or inverse-rendering experiments.

**Willems, S. "Shaders for Vulkan samples now also available in Slang." saschawillems.de, 2025; sample `shaders/slang/meshshader/meshshader.slang`.** [web] [recent]
https://www.saschawillems.de/blog/2025/06/03/shaders-for-vulkan-samples-now-also-available-in-slang/ — https://github.com/SaschaWillems/Vulkan/blob/master/shaders/slang/meshshader/meshshader.slang
Almost a hundred samples (170 shader files, 300+ SPIR-V modules) ported, including ray tracing and mesh shading; the mesh sample keeps `[shader("amplification")]` (`DispatchMesh`), `[shader("mesh")]` (`SetMeshOutputCounts`, `out indices`, `out vertices`) and `[shader("fragment")]` in one file. Willems advises the latest Slang release over the SDK copy, which "is missing some crucial fixes."
*Bearing:* Verified task+mesh Slang source for Vulkan; pin Forge's own `slangc` release.

**Legnitto, C. "Porting GPU shaders to Rust 30x faster with AI." rust-gpu.github.io, 2025; Rust-GPU/rust-gpu.** [web] [recent]
https://rust-gpu.github.io/blog/2025/06/24/vulkan-shader-port/ — https://github.com/Rust-GPU/rust-gpu
About 90% of Willems' samples were ported to Rust; vertex, fragment, tessellation, geometry, compute, mesh/task and ray-tracing stages worked, but `SPV_KHR_physical_storage_buffer` (buffer device address), sparse residency and fragment shading rate were unsupported (Jun 2025), and the README still calls the project "at an early stage" and "not yet production-ready" after Embark's 2024 hand-over. HLSL via DXC and GLSL via glslang remain the conservative alternatives without Slang's modules or single-source multi-stage files.
*Bearing:* Missing buffer device address disqualifies rust-gpu for a BDA-centric pipeline; Slang is the choice, GLSL only for third-party snippets.

## 10. Shipped precedents

**Remedy Entertainment. Alan Wake 2 (Northlight), 2023.** [web] [recent]
https://en.wikipedia.org/wiki/Alan_Wake_2 — https://www.dsogaming.com/news/alan-wake-2-is-one-of-the-first-games-to-require-mesh-shaders/
Released 27 Oct 2023 as "the first to be released with native support for mesh shaders": Remedy said the game "will require graphics cards that support mesh shaders," excluding GTX 10-series and RX 5000; on such GPUs it ran with "poor performance and visual errors," and an early-2024 patch improved GTX 10-series performance without changing the requirement. Press coverage of Remedy's talks describes Northlight's move to GPU-driven rendering with mesh shaders and pixel-accurate occlusion culling (reported, not verified against a Remedy primary source).
*Bearing:* Proof that a mesh-shader-first pipeline ships and that a slow fallback is commercially unacceptable; Forge's compute fallback must be a real path.

**id Software. Doom: The Dark Ages (id Tech 8), 2025.** [web] [recent]
https://store.steampowered.com/api/appdetails?appids=3017860 — https://www.resetera.com/threads/digital-foundry-inside-doom-the-dark-ages-creating-id-tech-8-interview-with-id-software.1192350/ — https://developer.nvidia.com/blog/how-id-software-used-neural-rendering-and-path-tracing-in-doom-the-dark-ages/ — https://en.gamegpu.com/test-gpu/action-fps-tps/doom-the-dark-ages-itogovyj-godovoj-benchmark-2025
Released May 2025 on a Vulkan-only id Tech 8; the official Steam requirements demand an "NVIDIA or AMD hardware Raytracing-capable GPU with 8GB dedicated VRAM or better (examples: NVIDIA RTX 2060 SUPER or better, AMD RX 6600 or better)." Digital Foundry's interview with engine director Billy Khan has segments titled "Without RT – This Game isn't Possible" and "Geometry Rendering – Something Different?", and a September 2025 patch added path tracing whose biggest wins came from opacity micromaps and shader execution reordering. Claims that id Tech 8 uses Nanite-like virtualized geometry are press inference from the absence of LOD pops, not confirmed by id.
*Bearing:* A hardware-RT-mandatory, Vulkan-only AAA engine validates Forge's Vulkan plus ray-query choice; the OMM/SER data apply directly to the 5070 Ti.

**Lopez, N. "Rendering 'Assassin's Creed Shadows'." GDC 2025; Berenguier, L., Minnetian, J. "Micropolygon Rendering in 'Anvil'." GDC 2026 (Ubisoft).** [talk] [recent]
https://gdcvault.com/play/1035526/Rendering-Assassin-s-Creed-Shadows — https://gdcvault.com/play/1035671/Micropolygon-Rendering-in-Anvil — https://80.lv/articles/gdc-2025-talk-rendering-assassin-s-creed-shadows
Shadows' Anvil iteration is presented around its "gpu driven pipeline" and the series' first ray-traced global illumination; the 2026 talk describes Micropolygon, Ubisoft's own virtualized geometry (continuous LOD with massive instancing, no authored LODs, no popping) that first shipped in Shadows, covering rasterization improvements, deferred-material extensions and GPU-driven streaming, in the lineage of Assassin's Creed Unity's cluster pipeline.
*Bearing:* A second independent, shipping Nanite-class system in an open-world engine whose deferred-material and streaming choices match the design below.

**Cloud Imperium Games. Star Citizen "Gen12" renderer and Vulkan.** [web] [recent]
https://starcitizen.tools/Gen12 — https://starcitizen.tools/Vulkan
Community documentation records Gen12 as a Vulkan-based renderer built to cut render-thread submission cost in draw-call-heavy cities, rolled out in parts (post-processing in Alpha 3.14, static geometry in 3.17, all scene geometry in 3.18) alongside the Vulkan back end that is also a prerequisite for DLSS and VR. No cluster, culling or LOD internals are public.
*Bearing:* A cautionary precedent: multi-year incremental migration is the cost of retrofitting a GPU-driven renderer onto a legacy engine, which Forge avoids by starting there.

**Rockstar Games. RAGE (Red Dead Redemption 2, Grand Theft Auto VI).** [web] [still-current]
https://en.wikipedia.org/wiki/Rockstar_Advanced_Game_Engine — https://imgeself.github.io/posts/2020-06-19-graphics-study-rdr2/
Publicly, RAGE is known only through RDR2's PC port (Vulkan default plus DirectX 12) and third-party captures: a 2020 RenderDoc study shows a conventional deferred renderer with six G-buffer targets, four-cascade shadows, per-frame environment cubemaps and a footprint displacement map for parallax occlusion mapping, with no visible cluster-culling or indirect-draw architecture. For GTA VI the encyclopedia entry lists platforms and a 2026 date and no rendering technology; Rockstar has published no engine talks or papers.
*Bearing:* Nothing to copy; do not cite RAGE as a design reference.

---

## Recommendation for Forge

**Build order.** Phase 0 (two weeks): the bindless substrate. Vulkan 1.4 minimum, dynamic rendering, synchronization2, one timeline semaphore per queue, a dedicated transfer queue relying on 1.4's streaming-transfer guarantee. All geometry, instance, cluster and material data live in device-address buffers; textures and storage buffers sit in one global update-after-bind descriptor-indexed set; every resource is an index in that set so a later move to `VK_EXT_descriptor_heap` (or descriptor buffers) is a back-end change. Slang is the only shading language, compiled to SPIR-V 1.6 with a pinned `slangc` release (not the SDK copy), one module per pipeline family, task+mesh+fragment in one file as in Willems' sample. Skip shader objects and device-generated commands until a profiler asks for them.

Phase 1 (first quarter): the cluster pipeline without LOD. Cook every mesh into clusters of at most 128 triangles and 128 vertices (`meshopt_buildMeshletsFlex`, minimum 96 triangles so clusters stay full), each with a bounding sphere, normal cone, positions quantized to the cluster bounds, strip-ordered indices and an 8-byte header; 128 is the largest unit inside both NVIDIA's (32 invocations, 4 primitives per lane) and AMD's (128 invocations, 1 per lane) comfort zones and gives Nanite's 7-bit triangle id. Runtime, entirely GPU-driven: (1) instance culling in compute (frustum, distance, previous-frame HZB reprojected); (2) cluster culling in compute (frustum, cone backface, HZB) writing a compacted cluster list; (3) rasterization into a 64-bit visibility buffer (depth in the high 32 bits, 25-bit cluster and 7-bit triangle in the low 32) through a mesh shader that reads clusters by device address and emits at most 128 vertices and primitives; (4) two-pass occlusion: draw clusters visible last frame, build the HZB, test the remainder against it; (5) material classification and per-material shading of the visibility buffer with analytic derivatives (`VK_KHR_fragment_shader_barycentric` or `VK_KHR_compute_shader_derivatives`). The task shader is used only for amplification (grass, clutter) and per-instance cluster ranges; per-cluster culling stays in compute so the fallback shares the code.

Phase 2: continuous LOD and streaming. Build the DAG offline with meshoptimizer's `clusterlod.h` through a thin FFI (port to Rust later on the `meshopt` crate's `partition_clusters` and `simplify_with_attributes_and_locks` if the FFI becomes a burden): groups of 4–8 clusters simplified to half with group-border locks, error propagated monotonically up the DAG in object space (`simplifyScale`), METIS added only if the DAG fails to reach a single root on test assets. Select LOD on the GPU with the parent/self error test against a screen-space threshold; add the compute software rasterizer for clusters under 64x64 px using 64-bit image atomics; page clusters from disk through a GPU request buffer (the `vk_lod_clusters` streaming design). Track two build metrics from Bevy's experience: fraction of DAG roots reached and mean cluster fill.

Phase 3: the same clusters everywhere else. Cluster acceleration structures (`VK_NV_cluster_acceleration_structure`) for ray queries where available, else a coarse-LOD BLAS; cluster tessellation and displacement in compute following `vk_tessellated_clusters`, no tessellation stage or micromaps; assemblies (micro-instanced repeated parts) and a voxel far-field for foliage and distant procedural terrain; planetary terrain via concurrent binary trees; skinning at cluster level with rigid-bone bounds.

**Fallback path.** Same cook, same culling compute, same visibility buffer: without `VK_EXT_mesh_shader`, the cluster-culling pass appends one `VkDrawIndexedIndirectCommand` per visible cluster (cluster-local index buffers concatenated at cook time) and the frame issues one `vkCmdDrawIndexedIndirectCount`, with the vertex shader decoding the same quantized format. The software rasterizer (which needs 64-bit image atomics) is skipped there, so the fallback loses sub-pixel efficiency, not correctness.

**The demo that proves it.** A 16 km² procedural terrain patch with a GPU-placed million instances drawn from twenty scanned props (0.5–3 M triangles each: roughly a billion source triangles and 100–300 M candidate triangles per frame) plus GPU-amplified grass, at 1440p on the RTX 5070 Ti with a 120 fps target and the RTX 3080 at 60 fps, with debug views for HZB, LOD level, software/hardware raster split and cluster fill, and an automated pixel-diff between the mesh-shader path and the indirect-count fallback on the same frame. If that holds frame rate while flying at 300 m/s with streaming on, the geometry pipeline is done and everything above it can assume it.

## Checked and left out

- **Barczak, J., "Why Geometry Shaders Are Slow (Unless You're Intel)," 2015.** joshbarczak.com serves an expired TLS certificate, the geeks3d mirror returned an empty page, Hacker News rate-limited and archive.org is blocked from this environment. Existence, date (18 Mar 2015) and thesis (input-order output forces serialization) are confirmed only by search snippets, so it is not cited; Reed 2018 and the Khronos blog carry the same argument.
- **"Seamless" cluster-LOD papers and "Efficient GPU Rendering of Dynamic Nanite-like…"** No paper with either title surfaced; the real 2024–2026 follow-ups found are Kuth et al., Aokana, Virtualized 3D Gaussians and CuRast, all cited.
- **An NVIDIA "GPU Instancer" product.** Does not exist; GPU Instancer Pro is GurBu Technologies' Unity asset (verified on the Asset Store).
- **Haar & Aaltonen slides and the JCGT paper text.** Both PDFs were fetched (4.2 MB and 1.9 MB) but no PDF text extractor is installed here, so the cluster sizes and filter lists attributed to them (and to Wihlidal's deck) are from memory and marked as such.
- **Nanite 2021 slides.** The 16 MB PDF exceeds the fetch limit; verified through the course index page and López's frame capture.
- **id Tech 8 "virtualized geometry."** Press inference from Digital Foundry's observations; not confirmed by id Software.
- **UE 5.5 Nanite skeletal release-note text.** The official release-notes page did not surface the paragraph in fetches; the claim rests on Epic's Nanite documentation (skeletal meshes listed as supported), an Epic forum thread and a secondary news source.
- **Intel first-party mesh-shader guidance.** None reachable beyond Mesa ANV support (Kristóf).
- **`VK_EXT_descriptor_heap` release date and extension number.** The reference page lists Tobias Hector (AMD), revision 1 and "ratified," but the number and date it returned looked inconsistent, so they are omitted.
- **Kuth et al. 2024 venue.** Only the arXiv record was reachable; no conference venue is claimed.
- **Star Citizen Gen12 internals.** CIG's Spectrum forum requires login; only the community wiki was reachable.
- **Grand Theft Auto VI rendering.** No public technical material exists.
- **RTX 5070 Ti limits on vulkan.gpuinfo.org.** Not fetched (search budget exhausted); the limits used are the owner's stated driver values.
- **Semantic Scholar DOI records for Burns & Hunt and Cignoni et al.** The API rate-limited on every attempt; both papers are verified through the live JCGT PDF and the ISTI-CNR publication page instead, and Cignoni's IEEE DOI is therefore not quoted.

## Verification notes

No browser pane was used at any point, and no YouTube page was opened or fetched; every talk is confirmed through its GDC Vault page, the SIGGRAPH Advances course index, GPUOpen's event page or the publisher's own page. Verification used WebSearch (budget exhausted after roughly 45 queries) and WebFetch only.

Fetched and read (every URL listed in an entry was fetched unless noted): the NVIDIA, Khronos, GPUOpen, timur.hu, reedbeta, filmicworlds, elopezr, hhoppe, Activision Research, saschawillems, rust-gpu and johannesugb pages; the SIGGRAPH Advances course indexes for 2015, 2020, 2021 and 2024; the GDC Vault pages for HZD 2017, Ghost of Tsushima 2021, AC Shadows 2025 and Anvil Micropolygon 2026; Epic's Nanite, 5.4 release-note and Nanite Foliage pages plus an Epic forum thread and alternativeto for 5.5; the GitHub READMEs, files, releases and discussion listed under meshoptimizer, nvpro-samples, NVIDIAGameWorks, METIS, Rust-GPU, Slang and Willems; docs.rs for the meshopt and Bevy meshlet modules; JMS55's three posts and index; the arXiv abstracts and the ggx-research CBT page; the Semantic Scholar API record for Schied & Dachsbacher; the Slang targets and autodiff pages; the Vulkan spec versions appendix and both refpages; Wikipedia (Alan Wake 2, Doom: The Dark Ages, RAGE), dsogaming, ResetEra, the Steam store API, gamegpu, starcitizen.tools, imgeself and the Unity Asset Store. The vkguide GPU-driven chapter was consulted for the fallback design but is not an entry.

Quoted from memory and flagged in the text: the cluster size, cone culling and reprojected-HZB details of Haar & Aaltonen 2015; Wihlidal's per-triangle filter list; the METIS partitioning step in Nanite's builder (the 128-triangle clusters, visibility-buffer packing and two-pass HZB are verified via López's capture). Dates, version numbers and measurements in the entries come from fetched pages. Where a fetch failed (Tom's Hardware, TechPowerUp, Windows Central, PCGamingWiki, Digital Trends, diglib.eg.org, KIT, unrealengine.com, the Steam store page, bethesda.net, web.archive.org, joshbarczak.com) the claim was re-verified elsewhere or moved to "Checked and left out."

## Implementation notes from Forge (2026-09-24)

Two pitfalls met while building the two-pass occlusion culling, both invisible in numbers
and in a plausible-looking picture, both found by pixel-diffing culling on against off
(`docs/demos/asteroids.md`):

- **A min-reduction sampler needs a linear filter.** `VK_EXT_sampler_filter_minmax` applies
  the reduction to the texels the filter would read. With NEAREST that is one texel, so a
  pyramid built through such a sampler is a point-sampled downscale, not a minimum, and the
  occlusion test reads one arbitrary depth of the rectangle. Use LINEAR mag/min filters (the
  mip level is chosen explicitly with `SampleLevel`).
- **One centred tap at `floor(log2(size))` is not conservative.** A bilinear footprint is two
  texels wide but only the texel-wide window around the sample point is guaranteed to be
  inside it. Either round the level up, or take four taps at the rectangle's corners at the
  floor level: a rectangle narrower than two texels spans at most three, and the two corner
  footprints always include the middle one. Forge uses the four corners (tighter: the bench
  culls 816 k meshlets instead of 790 k with the rounded-up level).
- **meshoptimizer's wide-cone convention.** `meshopt_computeMeshletBounds` sets
  `cone_cutoff = 1` and leaves `cone_axis = 0` when the normal cone exceeds a hemisphere
  (`mindp <= 0.1`). Normalising that axis on the GPU yields NaN; every comparison with NaN is
  false and the cluster is culled forever. Skip the cone test when `cone_cutoff >= 1`. On a
  rough procedural asteroid 12 % of the clusters are wide cones.
- **Jitter and culling.** With a TAA-jittered drawing projection, the pyramid holds the
  jittered image; the culling rectangle (computed from the unjittered camera) must be
  shifted by the jitter before sampling, or sub-texel rectangles fall outside the footprint.

Honest numbers after the fixes, same bench (1152 rocks, 127 M triangles, roughness 0.35):
30 M triangles drawn at 2.16 ms with occlusion versus 106 M at 6.30 ms without; the 13 M at
1.14 ms reported earlier was measured with a quarter of the visible geometry missing.

### The cluster LOD DAG in Forge (2026-09-24)

Built as the papers describe and as meshoptimizer's cluster LOD recipe does it: groups of
eight clusters (`meshopt_partitionClusters`), vertices shared between groups locked,
`meshopt_simplifyWithAttributes` with the lock array to half the triangles, re-clustering,
until one cluster remains (9–13 levels for the asteroids, about 2× the leaf triangles in the
tables, vertices shared across levels). Per cluster: the producing group's sphere and error
(`self`) and the consuming group's (`parent`, infinite for roots); errors and spheres are
made monotonic on the CPU, so `project(parent) > t >= project(self)` is a crack-free cut on
the GPU. Two additions that mattered on 3000 instances: an exact task-group table (every
instance's real cluster count, per-instance visibility bits) instead of "largest mesh ×
instances", and a per-mesh per-level table (min self error, max parent error, sphere reach)
so a task group of 32 clusters can prove that none of its levels can be selected at the
instance's distance and exit before reading a cluster. Ballad: 78 M → 0.62 M triangles, GPU
5.5 → 1.09 ms; bench 2.18 → 0.58 ms; every A/B (occlusion vs brute force, window on vs off,
LOD off vs the pre-DAG image) at 0 pixels. What remains in the geometry passes is the
task-shader walk itself (145 k groups per pass), which is the cluster-hierarchy work.

**Instance cull pass (issue #4, same day).** With exact tables the two meshlet passes were
still launch-bound: 145 k task groups each, 0.45 ms apiece whatever was drawn. A compute pass
with one thread per instance (frustum test, the per-level LOD window evaluated once per
level, task groups of the possible levels appended to a work list through an atomic counter
that is also the `x` of the indirect mesh-task command) leaves a few thousand groups per
pass; both passes draw `vkCmdDrawMeshTasksIndirectEXT` over that list. Geometry passes:
0.46 + 0.42 ms → 0.06 + 0.02 ms, cull pass 0.02 ms; ballad frame 0.34 ms at 1 px. This is
the GPU-driven shape the fallback path (compute culling + indirect count draws) will share.

### The render graph in Forge (2026-09-24, issue #1, D-020)

Everything above now draws through `forge_gpu::graph`. What the migration taught:

- **Declared accesses find the barriers hands miss.** Writing the plan down as data made
  three of the old hand barriers visibly redundant (a full `ALL_COMMANDS` wait on the depth
  buffer every frame, the "unmarked demo work" zone, the overlay's `ALL_COMMANDS` fence) and
  one subtle: the swapchain image went undefined → colour attachment → transfer destination
  every frame although its first use is the blit. The graph goes straight to the first use.
- **Per-mip states are the right granularity for a pyramid.** Eleven passes reading level
  `l−1` and writing level `l` produce two image barriers each, one profiler zone for all
  (consecutive passes with one label share a zone), and the pass-2 read of the whole pyramid
  then costs exactly one barrier (the last level written) because the other levels are
  already in the read state.
- **Cross-frame state is the point.** The visibility bits (task read/write every frame), the
  depth pyramid (read by the next frame when culling is frozen), the TAA histories (written
  one frame, read the next) and the HDR colour transient (read by the resolve, rewritten by
  the next frame's sky) all get their barrier from the state the previous frame left, not
  from a conservative wait. Transients start from `UNDEFINED` every frame but keep the
  stage/access of whatever last touched their memory, in either frame in flight.
- **Aliasing found its first customer the same day.** At the migration colour, depth and
  motion vectors were all alive at the resolve, so the heap equalled the sum (25.6 MB).
  With the visibility buffer (below) the id image dies at the resolve and the motion
  vectors are born after it, so they share memory: 32 MB requested, 25.6 MB heap; in the
  bench the colour image reuses the depth buffer (25.6 → 19.2 MB).
- **The bindless set and layouts.** A sampled-image descriptor carries a layout, so the
  graph asks the image which layout its `Sampled` reads use (`GENERAL` for storage-capable
  images such as the pyramid, `SHADER_READ_ONLY_OPTIMAL` otherwise) instead of the pass.
- **Cost.** Compiling and recording 19 passes moved the CPU record zone from 0.07 to
  0.08 ms; the GPU frame is unchanged (0.33 ms against 0.34 before), which is expected since
  the same work runs behind barriers of the same kinds.

### The visibility buffer in Forge (2026-09-24, issue #6)

Steps (3) and (5) of the Phase 1 plan above on the hardware path, minus the 64-bit target
and the per-material passes, which come with the software rasteriser (#3) and material
classification (#20). What was learned:

- **A 32-bit id needs a per-frame visible list.** Nanite packs 25 bits of cluster and 7 of
  triangle because its clusters live in one global table. Forge's clusters are per mesh and
  drawn per instance, so `(instance, cluster)` does not fit; the task shader appends each
  drawn cluster to a per-frame list (one atomic per task group, `WaveReadLaneAt` for the
  base, 1 M entries with an overflow counter in the statistics) and the id is
  `slot << 7 | triangle`. The list doubles as the record of what was drawn, which the
  material classification pass will read.
- **`SV_PrimitiveID` in the fragment stage costs a feature bit.** Slang emits the SPIR-V
  `Geometry` capability for it, and the validation layer then requires
  `VkPhysicalDeviceFeatures::geometryShader` although no geometry shader exists. Enabling
  the feature (every desktop GPU has it) beats routing the id through a mesh-shader
  per-primitive output that the fragment stage reads as a flat varying.
- **Analytic barycentrics are a dozen lines** (Schied & Dachsbacher 2015 as Hable writes
  it): `b_i / w_i` is affine on screen, so its value at vertex 0 and its two gradients give
  it at any pixel; the sum is `1 / w`; `lambda_i = w · b_i / w_i`; the derivatives follow by
  the quotient rule. Mirrored on the CPU with a unit test (recovery within 2e-5, gradients
  against finite differences), which is how the sign conventions of the Y-up NDC and the
  jittered projection were pinned down without staring at pixels.
- **Slivers disagree by design.** The hardware interpolator works from vertex positions
  snapped to the sub-pixel grid; the analytic form uses the exact float positions. On
  near-degenerate triangles at silhouettes the two round differently: 39 of 1.44 M pixels by
  more than two levels in the ballad, isolated, max 36 levels. Every culling A/B stays at 0
  because both sides of those comparisons go through the same resolve.
- **Cost at this scale is a small loss.** Mesh pass 1 0.07 → 0.06 ms (positions only),
  resolve 0.03 ms, frame 0.27 → 0.30 ms; bench 0.15 → 0.18. One normal and one light are
  too cheap for deferred shading to win; Hable measured the crossover at 8–10 px triangles
  with a real material graph. The step buys one shading path for three rasterisers and the
  entry point for materials, not milliseconds.

### Compute culling and the indirect-count fallback (issue #5)

What building the "Fallback path" above taught (numbers in `docs/demos/meshlets.md`):

- **The fallback needs no second cook.** With `VK_KHR_index_type_uint8` the cooked meshlet
  triangle lists are the index buffer as they are: a draw's `firstIndex` is the cluster's byte
  offset, its `vertexOffset` the cluster's window of the meshlet vertex table (so an index
  plus the offset lands in that table, which the vertex shader reads), and its
  `firstInstance` the cluster's slot in the visible list, which carries the id. The only new
  memory is the commands (20 B per listed cluster). It needs `drawIndirectFirstInstance`,
  `multiDrawIndirect`, `drawIndirectCount` and, for a fragment shader reading
  `SV_PrimitiveID`, `geometryShader` enabled; the primitive id restarts at 0 in every draw
  of an indirect-count call, so it is the triangle's index in its cluster.
- **Slang's `SV_VertexID` and `SV_InstanceID` are the D3D ones.** Compiled for Vulkan they
  subtract `BaseVertex` / `BaseInstance` (the SPIR-V shows the `OpISub` and the
  `DrawParameters` capability), which silently drops a `firstInstance` payload.
  `SV_VulkanVertexID` and `SV_VulkanInstanceID` are the raw `VertexIndex` and `InstanceIndex`.
- **Slang's direct SPIR-V emitter drops `precise`.** No `NoContraction` decoration comes out
  (the `-emit-spirv-via-glsl` route does emit it), so two inlined copies of the same float
  formula may be fused differently by the driver. It was a false lead here (the cut through
  the LOD DAG stayed exact), but anything that relies on two evaluations agreeing to the bit
  must share one code path.
- **The draw order is part of the output.** Two triangles can meet a sample at exactly the
  same depth, and a `GREATER_OR_EQUAL` depth test keeps the one drawn last. A visible list
  filled by racing atomics therefore draws in a different order every run and changes the
  image; only TAA's history accumulates the rare ties into a visible difference (half the runs
  of the ballad's golden came out about 3 000 pixels apart; the old task path's work-list
  order happened to be stable). The culls now append with a single-pass prefix sum over
  workgroups, the decoupled look-back of Merrill & Garland ("Single-pass Parallel Prefix Scan
  with Decoupled Look-back", NVIDIA technical report NVR-2016-002, 2016,
  https://research.nvidia.com/publication/2016-03_single-pass-parallel-prefix-scan-decoupled-look-back),
  with the workgroup's place taken from a ticket counter so that every predecessor is already
  running. Its cost is per workgroup (a ticket, a status word, a look-back read, on few
  addresses), so batching work items matters: one work item per workgroup cost 0.036 ms over the
  atomic version's 0.170 ms frame, eight per workgroup 0.011 ms, which is the task path's 0.18.
- **The look-back assumes forward progress between workgroups**, which Vulkan does not
  promise. It held on the RTX 5070 Ti, the only GPU tested so far; on GPUs without such guarantees
  (Apple M-series is the documented case) a spinning workgroup can starve the one it waits
  for. "Decoupled Fallback" (Smith, Levien & Owens, SPAA 2025; the companion repository
  https://github.com/b0nes164/GPUPrefixSums describes it as letting such devices run the scan
  "without crashing") bounds the spin and lets the waiting workgroup compute the missing
  count itself (issue #28). The paper's PDF could not be read here (no text extractor), so
  its details come from the ACM listing and the repository.
- **Indirect grids are two-dimensional.** The spec guarantees only 65 535 workgroups per
  dimension for compute and mesh dispatches; the culls write `x = min(n, 32 768)` and
  `y = ⌈n / 32 768⌉` and the rest of the last row exits.

### Sizing the visible list (issue #27)

- **Grow from the overflow count, reserve when the demand is known.** The cull already counts
  the clusters that do not fit; read back two frames later, that count sizes the list (the
  power of two above 1.5 × the demand, each frame slot replaced when its slot comes up). The
  LOD views need 4–15 k slots, so the list now starts at 65 536 instead of a fixed 1 M.
  The only exact bound comes from the scene: with LOD off a frame lists at most every
  instance's finest clusters, and the demos reserve that up front.
- **Holes are not transient when the exposure is automatic.** Two frames with holes at
  start-up change the luminance histogram, and the exposure's adaptation carries that for
  seconds: at full detail, a list grown after two frames and one reserved from the start gave
  frame 240 images 1.13 M pixels apart (up to 3 levels, TAA off). Any culling A/B at full
  detail is only exact when neither side ever dropped a cluster.

### The software rasteriser in Forge (issue #3)

- **Route by density, not size.** Nanite and Bevy send clusters under a pixel size to
  compute. On the RTX 5070 Ti, at a 1 px LOD error, clusters under 16–32 pixels still hold
  triangles of several pixels. The hardware draws them for almost nothing: pass 1 stayed at
  0.040 ms with a third of its clusters removed. What the hardware pays for is primitives per
  pixel, so a cluster is dense when its bounding rectangle holds fewer than 2 pixels per
  triangle.
- **A fixed cost that only volume repays.** A raster pass plus a merge cost about 0.02 ms.
  At full detail the software path halves the geometry (29–106 M dense triangles), but at
  the LOD views (0.01–0.08 M) it only adds time. The cull therefore counts dense triangles
  every frame, and the renderer runs the path from 1.5 M until the count falls below 0.75 M.
  Measured break-even is near 1 M. Both paths give the same pixels, so the switch cannot show.
- **The unified 64-bit target did not pay here.** Nanite's design has every rasteriser write
  `depth << 32 | id` with atomics, then export the depth. Built as planned, the fragment
  atomic cost 0.025 ms over the colour write through the ROPs, the export 0.016 and the
  clear 0.007. That made the LOD views 0.05 ms slower (bench 0.19 → 0.24 ms).
  - The hardware now keeps its 32-bit id and depth test.
  - The software rasteriser checks the hardware's pixel before its atomic.
  - A merge pass writes the winners in.
  - One tie rule serves every path, because the hardware draws in id order: nearer, or at
    equal depth the larger id.
  - Neither a tiled 64-bit layout nor a read before the atomic helped.
- **Matching the hardware's coverage.** Round-to-nearest-even to 1/256 pixel and the top-left
  rule match NVIDIA's rasteriser except for a handful of edge pixels per frame at full detail
  (19 of 1.44 M, 9 beyond one level). GLSL `Round` compiles to round-half-to-even there.
  Those pixels come from positions the fixed-function unit rounds to the other step, not
  from depth: a 1e-6 depth nudge moved none. The depth values differ in the last bits, which
  TAA's reprojection turns into sub-level differences over the frame.
- **The fallback gains most.** Its per-cluster indexed draws are what compute replaces: the
  bench at full detail goes 3.92 → 1.20 ms, against 2.27 → 1.15 on the mesh path.
