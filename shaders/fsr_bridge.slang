// FSR 3.1's upscaler in Forge (D-070, docs/research/fsr-3-1.md): the callbacks AMD's pass
// headers read and write through, over Forge's bindless images. Included by the eight pass
// files (`fsr_*.slang`), each after AMD's binding numbers of its pass (`FSR3UPSCALER_BIND_*`),
// which here index the pass's push constants: `srv[]` holds the sampled images' bindless
// handles, `uav[]` the storage images'. The constants AMD keeps in its constant buffers are a
// block of the frame (`FsrFrame`), written by `crates/forge-render/src/fsr.rs`.
//
// In place of AMD's `ffx_fsr3upscaler_callbacks_hlsl.h` (FidelityFX SDK v1.1.4, MIT; the
// headers it serves are in `third-party/fidelityfx/`). The options AMD's SDK compiles as
// permutations are fixed: 32-bit floats (no half), HDR input, motion vectors at render size
// without jitter, reversed depth, the reference Lanczos (no table), and the single-pass
// downsampler without wave operations (no assumption on the subgroup size).

import bindless;

#define FFX_GPU 1
#define FFX_HLSL 1
#define FFX_HLSL_SM 62
#define FFX_HALF 0
#define FFX_SPD_NO_WAVE_OPERATIONS 1
// Xbox-only in AMD's headers; defined so the preprocessor does not warn.
#define __XBATG_EXTRA_16_BIT_OPTIMISATION 0
#define FFX_PREFER_WAVE64
#define FFX_FSR3UPSCALER_EMBED_ROOTSIG_CONTENT
#define FFX_FSR3UPSCALER_EMBED_CB2_ROOTSIG_CONTENT
#define FFX_FSR3UPSCALER_OPTION_UPSAMPLE_SAMPLERS_USE_DATA_HALF 0
#define FFX_FSR3UPSCALER_OPTION_ACCUMULATE_SAMPLERS_USE_DATA_HALF 0
#define FFX_FSR3UPSCALER_OPTION_REPROJECT_SAMPLERS_USE_DATA_HALF 0
#define FFX_FSR3UPSCALER_OPTION_POSTPROCESSLOCKSTATUS_SAMPLERS_USE_DATA_HALF 0
#define FFX_FSR3UPSCALER_OPTION_UPSAMPLE_USE_LANCZOS_TYPE 2
#define FFX_FSR3UPSCALER_OPTION_REPROJECT_USE_LANCZOS_TYPE 0
#define FFX_FSR3UPSCALER_OPTION_HDR_COLOR_INPUT 1
#define FFX_FSR3UPSCALER_OPTION_LOW_RESOLUTION_MOTION_VECTORS 1
#define FFX_FSR3UPSCALER_OPTION_JITTERED_MOTION_VECTORS 0
#define FFX_FSR3UPSCALER_OPTION_INVERTED_DEPTH 1
#ifndef FFX_FSR3UPSCALER_OPTION_APPLY_SHARPENING
#define FFX_FSR3UPSCALER_OPTION_APPLY_SHARPENING 0
#endif

#include "third-party/fidelityfx/fsr3upscaler/ffx_fsr3upscaler_resources.h"
#include "third-party/fidelityfx/ffx_core.h"

// FSR's images of other formats than the bindless module's views, at the same binding. Two are
// read back across workgroups by the single-pass downsampler: coherent.
[[vk::binding(1, 0)]] [[vk::image_format("r16f")]] RWTexture2D<float> fsr_storage_r16f[];
[[vk::binding(1, 0)]] [[vk::image_format("r8")]] RWTexture2D<float> fsr_storage_r8[];
[[vk::binding(1, 0)]] [[vk::image_format("rgba8")]] RWTexture2D<float4> fsr_storage_rgba8[];
[[vk::binding(1, 0)]] [[vk::image_format("rgba32f")]] RWTexture2D<float4> fsr_storage_rgba32f[];
[[vk::binding(1, 0)]] [[vk::image_format("r32ui")]] RWTexture2D<uint> fsr_storage_r32ui[];
[[vk::binding(1, 0)]] [[vk::image_format("rg16f")]] globallycoherent RWTexture2D<float2> fsr_storage_rg16f_coherent[];
[[vk::binding(1, 0)]] [[vk::image_format("r32ui")]] globallycoherent RWTexture2D<uint> fsr_storage_r32ui_coherent[];

// AMD's constant buffers, in their order (cbFSR3Upscaler, cbSPD, cbRCAS).
struct FsrFrame
{
    int2 render_size;
    int2 previous_render_size;
    int2 upscale_size;
    int2 previous_upscale_size;
    int2 max_render_size;
    int2 max_upscale_size;
    float4 device_to_view_depth;
    float2 jitter;
    float2 previous_jitter;
    float2 motion_vector_scale;
    float2 downscale_factor;
    float2 motion_vector_jitter_cancellation;
    float tan_half_fov;
    float jitter_sequence_length;
    float delta_time;
    float delta_pre_exposure;
    float view_space_to_meters;
    float frame_index;
    float velocity_factor;
    float reactiveness_scale;
    float shading_change_scale;
    float accumulation_added_per_frame;
    float min_disocclusion_accumulation;
    uint spd_mips;
    uint spd_work_groups;
    uint pad;
    uint2 spd_work_group_offset;
    uint2 spd_render_size;
    uint4 rcas_config;
};

struct FsrPush
{
    FsrFrame* frame;
    uint srv[12];
    uint uav[12];
};

[[vk::push_constant]]
ConstantBuffer<FsrPush> fsr_push;

#define FSR_SRV(slot) NonUniformResourceIndex(fsr_push.srv[slot])
#define FSR_UAV(slot) NonUniformResourceIndex(fsr_push.uav[slot])
#define FSR_LINEAR g_samplers[SAMPLER_LINEAR_CLAMP]

// ---- Constants --------------------------------------------------------------------------------

FfxInt32x2 RenderSize() { return fsr_push.frame->render_size; }
FfxInt32x2 PreviousFrameRenderSize() { return fsr_push.frame->previous_render_size; }
FfxInt32x2 MaxRenderSize() { return fsr_push.frame->max_render_size; }
FfxInt32x2 UpscaleSize() { return fsr_push.frame->upscale_size; }
FfxInt32x2 PreviousFrameUpscaleSize() { return fsr_push.frame->previous_upscale_size; }
FfxInt32x2 MaxUpscaleSize() { return fsr_push.frame->max_upscale_size; }
FfxFloat32x2 Jitter() { return fsr_push.frame->jitter; }
FfxFloat32x2 PreviousFrameJitter() { return fsr_push.frame->previous_jitter; }
FfxFloat32x4 DeviceToViewSpaceTransformFactors() { return fsr_push.frame->device_to_view_depth; }
FfxFloat32x2 MotionVectorScale() { return fsr_push.frame->motion_vector_scale; }
FfxFloat32x2 DownscaleFactor() { return fsr_push.frame->downscale_factor; }
FfxFloat32x2 MotionVectorJitterCancellation() { return fsr_push.frame->motion_vector_jitter_cancellation; }
FfxFloat32 TanHalfFoV() { return fsr_push.frame->tan_half_fov; }
FfxFloat32 JitterSequenceLength() { return fsr_push.frame->jitter_sequence_length; }
FfxFloat32 DeltaTime() { return fsr_push.frame->delta_time; }
FfxFloat32 DeltaPreExposure() { return fsr_push.frame->delta_pre_exposure; }
FfxFloat32 ViewSpaceToMetersFactor() { return fsr_push.frame->view_space_to_meters; }
FfxFloat32 FrameIndex() { return fsr_push.frame->frame_index; }
FfxFloat32 VelocityFactor() { return fsr_push.frame->velocity_factor; }
FfxFloat32 AccumulationAddedPerFrame() { return fsr_push.frame->accumulation_added_per_frame; }
FfxFloat32 MinDisocclusionAccumulation() { return fsr_push.frame->min_disocclusion_accumulation; }
FfxUInt32 MipCount() { return fsr_push.frame->spd_mips; }
FfxUInt32 NumWorkGroups() { return fsr_push.frame->spd_work_groups; }
FfxUInt32x2 WorkGroupOffset() { return fsr_push.frame->spd_work_group_offset; }
FfxUInt32x2 SPD_RenderSize() { return fsr_push.frame->spd_render_size; }
FfxUInt32x4 RCASConfig() { return fsr_push.frame->rcas_config; }

// ---- Images -----------------------------------------------------------------------------------

#if defined(FSR3UPSCALER_BIND_SRV_SPD_MIPS)
FfxInt32x2 GetSPDMipDimensions(FfxUInt32 uMipLevel)
{
    uint width, height, levels;
    g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_SPD_MIPS)].GetDimensions(uMipLevel, width, height, levels);
    return FfxInt32x2(width, height);
}
FfxFloat32x2 SampleSPDMipLevel(FfxFloat32x2 fUV, FfxUInt32 mipLevel)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_SPD_MIPS)].SampleLevel(FSR_LINEAR, fUV, mipLevel).xy;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_INPUT_DEPTH)
FfxFloat32 LoadInputDepth(FfxUInt32x2 iPxPos)
{
    return g_sampled_images_r[FSR_SRV(FSR3UPSCALER_BIND_SRV_INPUT_DEPTH)].Load(int3(iPxPos, 0));
}
FfxFloat32 SampleInputDepth(FfxFloat32x2 fUV)
{
    return g_sampled_images_r[FSR_SRV(FSR3UPSCALER_BIND_SRV_INPUT_DEPTH)].SampleLevel(FSR_LINEAR, fUV, 0);
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_REACTIVE_MASK)
FfxFloat32 LoadReactiveMask(FfxUInt32x2 iPxPos)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_REACTIVE_MASK)].Load(int3(iPxPos, 0)).x * fsr_push.frame->reactiveness_scale;
}
FfxInt32x2 GetReactiveMaskResourceDimensions()
{
    uint width, height;
    g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_REACTIVE_MASK)].GetDimensions(width, height);
    return FfxInt32x2(width, height);
}
FfxFloat32 SampleReactiveMask(FfxFloat32x2 fUV)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_REACTIVE_MASK)].SampleLevel(FSR_LINEAR, fUV, 0).x * fsr_push.frame->reactiveness_scale;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_TRANSPARENCY_AND_COMPOSITION_MASK)
FfxFloat32 LoadTransparencyAndCompositionMask(FfxUInt32x2 iPxPos)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_TRANSPARENCY_AND_COMPOSITION_MASK)].Load(int3(iPxPos, 0)).x;
}
FfxInt32x2 GetTransparencyAndCompositionMaskResourceDimensions()
{
    uint width, height;
    g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_TRANSPARENCY_AND_COMPOSITION_MASK)].GetDimensions(width, height);
    return FfxInt32x2(width, height);
}
FfxFloat32 SampleTransparencyAndCompositionMask(FfxFloat32x2 fUV)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_TRANSPARENCY_AND_COMPOSITION_MASK)].SampleLevel(FSR_LINEAR, fUV, 0).x;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_INPUT_COLOR)
FfxFloat32x3 LoadInputColor(FfxUInt32x2 iPxPos)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_INPUT_COLOR)].Load(int3(iPxPos, 0)).rgb;
}
FfxFloat32x3 SampleInputColor(FfxFloat32x2 fUV)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_INPUT_COLOR)].SampleLevel(FSR_LINEAR, fUV, 0).rgb;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_INPUT_MOTION_VECTORS)
FfxFloat32x2 LoadInputMotionVector(FfxUInt32x2 iPxDilatedMotionVectorPos)
{
    FfxFloat32x2 fSrcMotionVector = g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_INPUT_MOTION_VECTORS)].Load(int3(iPxDilatedMotionVectorPos, 0)).xy;
    FfxFloat32x2 fUvMotionVector = fSrcMotionVector * MotionVectorScale();
#if FFX_FSR3UPSCALER_OPTION_JITTERED_MOTION_VECTORS
    fUvMotionVector -= MotionVectorJitterCancellation();
#endif
    return fUvMotionVector;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_INTERNAL_UPSCALED)
FfxFloat32x4 LoadHistory(FfxUInt32x2 iPxHistory)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_INTERNAL_UPSCALED)].Load(int3(iPxHistory, 0));
}
FfxFloat32x4 SampleHistory(FfxFloat32x2 fUV)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_INTERNAL_UPSCALED)].SampleLevel(FSR_LINEAR, fUV, 0);
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_LUMA_HISTORY)
void StoreLumaHistory(FfxUInt32x2 iPxPos, FfxFloat32x4 fLumaHistory)
{
    g_storage_images_rgba16f[FSR_UAV(FSR3UPSCALER_BIND_UAV_LUMA_HISTORY)][iPxPos] = fLumaHistory;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_LUMA_HISTORY)
FfxFloat32x4 LoadLumaHistory(FfxInt32x2 iPxPos)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_LUMA_HISTORY)].Load(int3(iPxPos, 0));
}
FfxFloat32x4 SampleLumaHistory(FfxFloat32x2 fUV)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_LUMA_HISTORY)].SampleLevel(FSR_LINEAR, fUV, 0);
}
#endif

// RCAS in [0, 1] (Forge's, D-070): its limiter assumes colours there, but FSR's history is
// pre-exposed HDR. Read as it is, a bright edge over 1 (a white wall in the sun) was left
// unsharpened while the darker detail was sharpened, which only added shimmer. The RCAS pass
// (FSR_RCAS_TONEMAPPED) reads the history through AMD's own reversible tone map, x / (1 + its
// largest channel), and writes back through the inverse.
FfxFloat32x3 fsr_rcas_tonemap(FfxFloat32x3 c)
{
    return sqrt(max(c, 0.0) / (max(max(0.0, c.r), max(c.g, c.b)) + 1.0));
}
FfxFloat32x3 fsr_rcas_inverse_tonemap(FfxFloat32x3 c)
{
    c *= c;
    return c / max(FFX_TONEMAP_EPSILON, 1.0 - max(c.r, max(c.g, c.b)));
}

#if defined(FSR3UPSCALER_BIND_SRV_RCAS_INPUT)
FfxFloat32x4 LoadRCAS_Input(FfxInt32x2 iPxPos)
{
    FfxFloat32x4 c = g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_RCAS_INPUT)].Load(int3(iPxPos, 0));
#if defined(FSR_RCAS_TONEMAPPED)
    c.rgb = fsr_rcas_tonemap(c.rgb);
#endif
    return c;
}
FfxFloat32x3 SampleRCAS_Input(FfxFloat32x2 fUV)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_RCAS_INPUT)].SampleLevel(FSR_LINEAR, fUV, 0).rgb;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_INTERNAL_UPSCALED)
void StoreReprojectedHistory(FfxUInt32x2 iPxHistory, FfxFloat32x4 fHistory)
{
    g_storage_images_rgba16f[FSR_UAV(FSR3UPSCALER_BIND_UAV_INTERNAL_UPSCALED)][iPxHistory] = fHistory;
}
void StoreInternalColorAndWeight(FfxUInt32x2 iPxPos, FfxFloat32x4 fColorAndWeight)
{
    g_storage_images_rgba16f[FSR_UAV(FSR3UPSCALER_BIND_UAV_INTERNAL_UPSCALED)][iPxPos] = fColorAndWeight;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_UPSCALED_OUTPUT)
void StoreUpscaledOutput(FfxUInt32x2 iPxPos, FfxFloat32x3 fColor)
{
#if defined(FSR_RCAS_TONEMAPPED)
    fColor = fsr_rcas_inverse_tonemap(fColor);
#endif
    g_storage_images_rgba16f[FSR_UAV(FSR3UPSCALER_BIND_UAV_UPSCALED_OUTPUT)][iPxPos] = FfxFloat32x4(fColor, 1.0);
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_ACCUMULATION)
FfxFloat32 SampleAccumulation(FfxFloat32x2 fUV)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_ACCUMULATION)].SampleLevel(FSR_LINEAR, fUV, 0).x;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_ACCUMULATION)
void StoreAccumulation(FfxUInt32x2 iPxPos, FfxFloat32 fAccumulation)
{
    fsr_storage_r8[FSR_UAV(FSR3UPSCALER_BIND_UAV_ACCUMULATION)][iPxPos] = fAccumulation;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_SHADING_CHANGE)
FfxFloat32 LoadShadingChange(FfxUInt32x2 iPxPos)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_SHADING_CHANGE)].Load(int3(iPxPos, 0)).x * fsr_push.frame->shading_change_scale;
}
FfxFloat32 SampleShadingChange(FfxFloat32x2 fUV)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_SHADING_CHANGE)].SampleLevel(FSR_LINEAR, fUV, 0).x * fsr_push.frame->shading_change_scale;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_SHADING_CHANGE)
void StoreShadingChange(FfxUInt32x2 iPxPos, FfxFloat32 fShadingChange)
{
    fsr_storage_r8[FSR_UAV(FSR3UPSCALER_BIND_UAV_SHADING_CHANGE)][iPxPos] = fShadingChange;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_FARTHEST_DEPTH)
FfxInt32x2 GetFarthestDepthResourceDimensions()
{
    uint width, height;
    g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_FARTHEST_DEPTH)].GetDimensions(width, height);
    return FfxInt32x2(width, height);
}
FfxFloat32 LoadFarthestDepth(FfxUInt32x2 iPxPos)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_FARTHEST_DEPTH)].Load(int3(iPxPos, 0)).x;
}
FfxFloat32 SampleFarthestDepth(FfxFloat32x2 fUV)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_FARTHEST_DEPTH)].SampleLevel(FSR_LINEAR, fUV, 0).x;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_FARTHEST_DEPTH)
void StoreFarthestDepth(FfxUInt32x2 iPxPos, FfxFloat32 fDepth)
{
    fsr_storage_r16f[FSR_UAV(FSR3UPSCALER_BIND_UAV_FARTHEST_DEPTH)][iPxPos] = fDepth;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_FARTHEST_DEPTH_MIP1)
FfxInt32x2 GetFarthestDepthMip1ResourceDimensions()
{
    uint width, height;
    g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_FARTHEST_DEPTH_MIP1)].GetDimensions(width, height);
    return FfxInt32x2(width, height);
}
FfxFloat32 LoadFarthestDepthMip1(FfxUInt32x2 iPxPos)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_FARTHEST_DEPTH_MIP1)].Load(int3(iPxPos, 0)).x;
}
FfxFloat32 SampleFarthestDepthMip1(FfxFloat32x2 fUV)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_FARTHEST_DEPTH_MIP1)].SampleLevel(FSR_LINEAR, fUV, 0).x;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_FARTHEST_DEPTH_MIP1)
void StoreFarthestDepthMip1(FfxUInt32x2 iPxPos, FfxFloat32 fDepth)
{
    fsr_storage_r16f[FSR_UAV(FSR3UPSCALER_BIND_UAV_FARTHEST_DEPTH_MIP1)][iPxPos] = fDepth;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_CURRENT_LUMA)
FfxFloat32 LoadCurrentLuma(FfxUInt32x2 iPxPos)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_CURRENT_LUMA)].Load(int3(iPxPos, 0)).x;
}
FfxFloat32 SampleCurrentLuma(FfxFloat32x2 uv)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_CURRENT_LUMA)].SampleLevel(FSR_LINEAR, uv, 0).x;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_CURRENT_LUMA)
void StoreCurrentLuma(FfxUInt32x2 iPxPos, FfxFloat32 fLuma)
{
    fsr_storage_r16f[FSR_UAV(FSR3UPSCALER_BIND_UAV_CURRENT_LUMA)][iPxPos] = fLuma;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_LUMA_INSTABILITY)
FfxFloat32 SampleLumaInstability(FfxFloat32x2 uv)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_LUMA_INSTABILITY)].SampleLevel(FSR_LINEAR, uv, 0).x;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_LUMA_INSTABILITY)
void StoreLumaInstability(FfxUInt32x2 iPxPos, FfxFloat32 fLumaInstability)
{
    fsr_storage_r16f[FSR_UAV(FSR3UPSCALER_BIND_UAV_LUMA_INSTABILITY)][iPxPos] = fLumaInstability;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_PREVIOUS_LUMA)
FfxFloat32 LoadPreviousLuma(FfxUInt32x2 iPxPos)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_PREVIOUS_LUMA)].Load(int3(iPxPos, 0)).x;
}
FfxFloat32 SamplePreviousLuma(FfxFloat32x2 uv)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_PREVIOUS_LUMA)].SampleLevel(FSR_LINEAR, uv, 0).x;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_NEW_LOCKS)
FfxFloat32 LoadNewLocks(FfxUInt32x2 iPxPos)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_NEW_LOCKS)].Load(int3(iPxPos, 0)).x;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_NEW_LOCKS)
FfxFloat32 LoadRwNewLocks(FfxUInt32x2 iPxPos)
{
    return fsr_storage_r8[FSR_UAV(FSR3UPSCALER_BIND_UAV_NEW_LOCKS)][iPxPos];
}
void StoreNewLocks(FfxUInt32x2 iPxPos, FfxFloat32 newLock)
{
    fsr_storage_r8[FSR_UAV(FSR3UPSCALER_BIND_UAV_NEW_LOCKS)][iPxPos] = newLock;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_RECONSTRUCTED_PREV_NEAREST_DEPTH)
FfxFloat32 LoadReconstructedPrevDepth(FfxUInt32x2 iPxPos)
{
    return asfloat(g_sampled_images_u32[FSR_SRV(FSR3UPSCALER_BIND_SRV_RECONSTRUCTED_PREV_NEAREST_DEPTH)].Load(int3(iPxPos, 0)));
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_RECONSTRUCTED_PREV_NEAREST_DEPTH)
void StoreReconstructedDepth(FfxUInt32x2 iPxSample, FfxFloat32 fDepth)
{
    // Reversed depth: the nearest is the largest (AMD's InterlockedMax for inverted depth).
    InterlockedMax(fsr_storage_r32ui[FSR_UAV(FSR3UPSCALER_BIND_UAV_RECONSTRUCTED_PREV_NEAREST_DEPTH)][iPxSample], asuint(fDepth));
}
void SetReconstructedDepth(FfxUInt32x2 iPxSample, const FfxUInt32 uValue)
{
    fsr_storage_r32ui[FSR_UAV(FSR3UPSCALER_BIND_UAV_RECONSTRUCTED_PREV_NEAREST_DEPTH)][iPxSample] = uValue;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_DILATED_DEPTH)
void StoreDilatedDepth(FFX_PARAMETER_IN FfxUInt32x2 iPxPos, FFX_PARAMETER_IN FfxFloat32 fDepth)
{
    g_storage_images_r32f[FSR_UAV(FSR3UPSCALER_BIND_UAV_DILATED_DEPTH)][iPxPos] = fDepth;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_DILATED_MOTION_VECTORS)
void StoreDilatedMotionVector(FFX_PARAMETER_IN FfxUInt32x2 iPxPos, FFX_PARAMETER_IN FfxFloat32x2 fMotionVector)
{
    g_storage_images_rg16f[FSR_UAV(FSR3UPSCALER_BIND_UAV_DILATED_MOTION_VECTORS)][iPxPos] = fMotionVector;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_DILATED_MOTION_VECTORS)
FfxFloat32x2 LoadDilatedMotionVector(FfxUInt32x2 iPxInput)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_DILATED_MOTION_VECTORS)].Load(int3(iPxInput, 0)).xy;
}
FfxFloat32x2 SampleDilatedMotionVector(FfxFloat32x2 fUV)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_DILATED_MOTION_VECTORS)].SampleLevel(FSR_LINEAR, fUV, 0).xy;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_DILATED_DEPTH)
FfxFloat32 LoadDilatedDepth(FfxUInt32x2 iPxInput)
{
    return g_sampled_images_r[FSR_SRV(FSR3UPSCALER_BIND_SRV_DILATED_DEPTH)].Load(int3(iPxInput, 0));
}
FfxFloat32 SampleDilatedDepth(FfxFloat32x2 fUV)
{
    return g_sampled_images_r[FSR_SRV(FSR3UPSCALER_BIND_SRV_DILATED_DEPTH)].SampleLevel(FSR_LINEAR, fUV, 0);
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_INPUT_EXPOSURE)
FfxFloat32 Exposure()
{
    FfxFloat32 exposure = g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_INPUT_EXPOSURE)].Load(int3(0, 0, 0)).x;
    return exposure == 0.0 ? 1.0 : exposure;
}
#endif

// The reference Lanczos needs no table (FFX_FSR3UPSCALER_OPTION_REPROJECT_USE_LANCZOS_TYPE 0).
FfxFloat32 SampleLanczos2Weight(FfxFloat32 x)
{
    return 0.0;
}

#if defined(FSR3UPSCALER_BIND_SRV_DILATED_REACTIVE_MASKS)
FfxFloat32x4 SampleDilatedReactiveMasks(FfxFloat32x2 fUV)
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_DILATED_REACTIVE_MASKS)].SampleLevel(FSR_LINEAR, fUV, 0);
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_DILATED_REACTIVE_MASKS)
void StoreDilatedReactiveMasks(FFX_PARAMETER_IN FfxUInt32x2 iPxPos, FFX_PARAMETER_IN FfxFloat32x4 fDilatedReactiveMasks)
{
    fsr_storage_rgba8[FSR_UAV(FSR3UPSCALER_BIND_UAV_DILATED_REACTIVE_MASKS)][iPxPos] = fDilatedReactiveMasks;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_FRAME_INFO)
FfxFloat32x4 LoadFrameInfo()
{
    return fsr_storage_rgba32f[FSR_UAV(FSR3UPSCALER_BIND_UAV_FRAME_INFO)][int2(0, 0)];
}
void StoreFrameInfo(FfxFloat32x4 fInfo)
{
    fsr_storage_rgba32f[FSR_UAV(FSR3UPSCALER_BIND_UAV_FRAME_INFO)][int2(0, 0)] = fInfo;
}
#endif

#if defined(FSR3UPSCALER_BIND_SRV_FRAME_INFO)
FfxFloat32x4 FrameInfo()
{
    return g_sampled_images[FSR_SRV(FSR3UPSCALER_BIND_SRV_FRAME_INFO)].Load(int3(0, 0, 0));
}
#endif

// The single-pass downsampler's six mips: storage slots FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_0
// to _5, the last coherent (the last workgroup reads it back).
#if defined(FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_0)
FfxFloat32x2 RWLoadPyramid(FFX_PARAMETER_IN FfxInt32x2 iPxPos, FFX_PARAMETER_IN FfxUInt32 index)
{
    if (index == 5u)
        return fsr_storage_rg16f_coherent[FSR_UAV(FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_5)][iPxPos];
    return g_storage_images_rg16f[FSR_UAV(FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_0 + min(index, 4u))][iPxPos];
}
void StorePyramid(FFX_PARAMETER_IN FfxInt32x2 iPxPos, FFX_PARAMETER_IN FfxFloat32x2 outValue, FFX_PARAMETER_IN FfxUInt32 index)
{
    if (index == 5u)
        fsr_storage_rg16f_coherent[FSR_UAV(FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_5)][iPxPos] = outValue;
    else if (index < 5u)
        g_storage_images_rg16f[FSR_UAV(FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_0 + index)][iPxPos] = outValue;
}
#endif

#if defined(FSR3UPSCALER_BIND_UAV_SPD_GLOBAL_ATOMIC)
void SPD_IncreaseAtomicCounter(inout FfxUInt32 spdCounter)
{
    InterlockedAdd(fsr_storage_r32ui_coherent[FSR_UAV(FSR3UPSCALER_BIND_UAV_SPD_GLOBAL_ATOMIC)][int2(0, 0)], 1, spdCounter);
}
void SPD_ResetAtomicCounter()
{
    fsr_storage_r32ui_coherent[FSR_UAV(FSR3UPSCALER_BIND_UAV_SPD_GLOBAL_ATOMIC)][int2(0, 0)] = 0;
}
#endif
