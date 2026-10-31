# NVIDIA's SHaRC and NRC under D-088's bounce: the SDKs as Forge would use them

*Research note, 2026-10-30, read from the SDKs fetched into the git-ignored `sharc-sdk/` (SHaRC
1.8.3, commit 4e21b585), `rtxgi-sdk/` (RTXGI 2.8.0: the guides and the path tracer sample) and
`nrc-sdk/` (NRC v0.16.0.0). Nothing of them enters the repository; the quotes are short and
the file names are the SDKs'. Not legal advice.*

## Why

The owner's decision of 2026-10-30 (D-088, "Amended: SHaRC replaces this port"): NVIDIA's SHaRC
lights the bounce's hits on every GPU, and NRC, NVIDIA's neural cache, gets a trial beside it.
The Solari/kajiya port stays behind `r.gi.cache`, off, as the reference and the fallback.

## SHaRC (Spatially Hashed Radiance Cache)

- **What it is:** a world-space hash grid of voxels holding the radiance that leaves surfaces,
  filled by a sparse path tracer and read where a path can stop early. Shader-only: three headers
  (`SharcCommon.h`, `HashGridCommon.h`, `SharcTypes.h`), no library, the NVIDIA RTX SDKs licence
  (they ship "incorporated in object code format", 1c; never standalone, 4b).
- **Three passes a frame** (`docs/Integration.md`):
  1. **Update:** a path tracer over a sparse subset of pixels (the guide: one random pixel per
     5 × 5 block, about 4 %, moving each frame). Each vertex calls `SharcUpdateHit` with its
     direct light and each miss `SharcUpdateMiss`. After each new direction, `SharcSetThroughput`
     gets the segment's throughput. The radiance goes back along the path (two vertices with
     resampling, the default) into the voxels, by integer atomic adds.
  2. **Resolve:** one compute thread per entry (`SharcResolveEntry`) blends the frame's sums into
     the resolved data (at most `accumulationFrameNum` frames, 32 in the sample) and evicts
     entries left without samples for `staleFrameNumMax` frames (64; never fewer than 8).
  3. **Query:** the renderer's own tracing reads `SharcGetCachedRadiance` at an indirect hit and
     stops there. Only where the segment just traced is longer than the voxel's diagonal
     (`voxelSize × √3`), and for glossy lobes only where the lobe's footprint is wider than the
     voxel. Otherwise it traces on.
- **The grid:** the voxel size follows the distance to the camera,
  `base^(level − bias) / sceneScale` with `level = log_base(distance) + bias` (at least 1).
  Base 2 with the sample's `sceneScale` 50 gives about 4 cm at 2 m and 20 cm at 10 m. Keys are
  64-bit: 17 bits a position axis, 9 for the level, 3 for the normal's octant. Slots are found in
  buckets of 16 by linear probing.
- **Memory:** 40 bytes a slot (8 key, 16 accumulation, 16 resolved); 2²² slots, the guide's
  baseline, take 160 MiB. A static camera uses 10–20 % of them. 64 bytes a slot with directional
  (SH) encoding.
- **Device:** native fp16 (`float16_t4` in the resolved data); 64-bit atomics for insertion
  (`HASH_GRID_ENABLE_64_BIT_ATOMICS`, else a lock buffer).
- **Options that matter for Forge** (all `#ifndef` defaults in the headers, set before including
  them; the headers stay unmodified):
  - `RW_STRUCTURED_BUFFER(name, type)` and `BUFFER_AT_OFFSET(name, offset)`: a bridge can make
    the buffers device-address pointers, Forge's convention (`T*` in the frame's block).
  - `SHARC_SEPARATE_EMISSIVE`: a vertex's own `emissive` is left out of its own voxel but goes
    back along the path, and the query adds the caller's value. Given the hit's sunlight (its
    shadow ray) as that term, the voxels hold only the light after the hit, and the bounce keeps
    exact sun shadows at its hits, as Forge's bounce has them today.
  - `SHARC_MATERIAL_DEMODULATION`: radiance divided by a factor (the albedo) when stored and
    multiplied back at the query, so textured hits keep their detail. The Solari port did the
    same by storing irradiance.
  - `SHARC_ENABLE_SH_ENCODING` (directional radiance) and `SHARC_ENABLE_RESPONSIVE_LIGHTING`
    (short-lived lights): not needed for a diffuse bounce at first.
  - `radianceScale`: the factor before the u32 sums (1e3 in the sample). Forge's sunlit
    radiances are large, so it is chosen from the scene and checked with
    `SharcDebugBitsOccupancyRadiance`.
- **Determinism (D-016):** the sums are integer atomic adds, so their order doesn't matter, and
  the resolve works entry by entry. Which slot a key takes in its bucket follows the threads'
  order, but a lookup finds it wherever it is. So the light should come out the same from run to
  run while no bucket overflows. To be checked, not assumed.
- **Patents:** NVIDIA's own, used as NVIDIA designed them, as RTXDI covers ReSTIR: an implied
  licence, no written grant (the owner's SDK rule, 2026-10-30). The hash grid is US 10,614,613.
  Reading the cache by the segment's length is US 11,790,594 (the reason the Solari port read
  at every hit). The Gautron families (US 11,609,899, US 12,394,144) are
  `surface-radiance-cache.md`'s. Forge's own code around SHaRC adds none of these mechanics.

## NRC (Neural Radiance Cache)

- **What it is:** a small network trained every frame on full-length training paths, which
  predicts the radiance leaving a point in a direction (glossy included). Binary only:
  `NRC_Vulkan.dll` (5.8 MB) with CUDA 13's runtime (`cudart64_13.dll`, `nvrtc64_130_0.dll`
  106 MB, `nvrtc-builtins64_134.dll`), headers and an import library. NVIDIA calls it
  experimental. Same licence.
- **Hardware:** Tensor Cores through CUDA, so NVIDIA RTX only (20 series on, its changelog). AMD
  keeps SHaRC.
- **Per frame:**
  1. an update pass of training paths at a low resolution (`ComputeIdealTrainingDimensions`:
     197 × 111 for 1080p with NRC 0.16's default four training batches a frame, 279 × 157 with
     eight; its guide's table gives 274 × 154; up to eight vertices);
  2. the query pass, full resolution, writing query records where paths stop;
  3. `QueryAndTrain` (inference and training, recorded into Forge's command buffer);
  4. a resolve (the library's, or Forge's own for a split signal);
  5. `EndFrame` on the queue.
- **Integration (as built, 2026-10-31):**
  - The API is C++ classes (`nrc::vulkan::Context`), so Rust needs a small C shim of Forge's own:
    `tools/forge_nrc.cpp`, built with MSVC against `NRC_Vulkan.lib` by `tools/fetch-nrc.sh` into
    `nrc-sdk/bin/forge_nrc.dll`, loaded at run time by `crates/forge-gpu/src/nrc.rs`.
  - No interop: NRC records its CUDA kernels into Vulkan's command buffers through
    `VK_NVX_binary_import`. What it asks for (NRC 0.16's lists): the device extensions
    `VK_NVX_binary_import`, `VK_NVX_image_view_handle`, `VK_EXT_memory_budget` and
    `VK_KHR_buffer_device_address`, the instance extension
    `VK_KHR_get_physical_device_properties2`, and `scalarBlockLayout` and
    `uniformBufferStandardLayout`. Without the two `KHR` extensions enabled by name (both core
    since Vulkan 1.1 and 1.2, which Forge uses without their names) its context fails with
    "Internal error setting up device".
  - The DLL's signature is checked (`VerifySignature`).
  - NRC allocates its buffers itself: five of the nine sit on CUDA's shared heap, so Forge does
    not make them; it wraps them as graph buffers it never frees (`Buffer::foreign`).
  - Slang lays a pointer's target out naturally (scalar), which is DXC's `-fvk-use-dx-layout`
    that NRC expects (`NrcRadianceParams`: 56 bytes, offsets 0, 12, 16, 24, 32, 44): its
    structured buffers become pointers (`NRC_RW_STRUCTURED_BUFFER(T) T*`) with no custom
    accessors, and `Nrc.hlsli` compiles under Slang 2026.20 unmodified.
  - Its training loss cannot be read back on Vulkan (`cudaMemcpyAsync` "operation not
    supported"): Forge passes none.
  - Its two counters sit on its read-back heap, where the host reads them: each wave's increment
    from Forge's bounce cost 1.4 ms at 1600 × 900 (the classroom's `gi/trace`, 0.52 ms without
    queries, 1.96 with). Forge counts in a buffer of its own in VRAM and copies the counts into
    NRC's before its inference (0.60 ms).
  - The validation layer: without its debug buffers NRC 0.16 still writes their descriptor, a null
    buffer (VUID-VkDescriptorBufferInfo-buffer-02998), so Forge turns them on (0.5 MiB); and it
    leaves a fence and some memory alive at `vkDestroyDevice` (VUID-vkDestroyDevice-device-05137).
    Both reproduce in a minimal C++ program that follows NVIDIA's guide, without Forge.
  - It writes `nrc_session_log.txt` where the program starts (git-ignored).
- **Memory:** its buffers take 171 MiB at 1080p with eight vertices (125 MiB at 1600 × 900),
  mostly the queries' records (112 MiB of `QueryRadianceParams` at 1080p); its guide's table says
  192 MB. With its network and workspace, the device-local heap grew by 267 MiB at 1080p.
- **Patents:** NVIDIA's neural cache family (US 11,610,360 and siblings, to about 2041), NVIDIA's
  own library used as designed, under the same rule.

## For Forge

D-088's design section ("SHaRC under the bounce: the design") turns this into passes. SHaRC comes
first, on every GPU. NRC follows on the same hook, NVIDIA only, behind a switch. It stays only if
it beats SHaRC against the path tracer where the probes and SHaRC fall short.
