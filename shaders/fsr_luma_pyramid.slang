// FSR 3.1's luma pyramid (D-070): AMD's binding numbers of its pass, the bridge, its headers.
// Driven by crates/forge-render/src/fsr.rs.

#define FSR3UPSCALER_BIND_SRV_CURRENT_LUMA 0
#define FSR3UPSCALER_BIND_SRV_FARTHEST_DEPTH 1
#define FSR3UPSCALER_BIND_UAV_SPD_GLOBAL_ATOMIC 0
#define FSR3UPSCALER_BIND_UAV_FRAME_INFO 1
#define FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_0 2
#define FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_1 3
#define FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_2 4
#define FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_3 5
#define FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_4 6
#define FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_5 7
#define FSR3UPSCALER_BIND_UAV_FARTHEST_DEPTH_MIP1 8

#include "fsr_bridge.slang"
#include "fsr3upscaler/ffx_fsr3upscaler_common.h"
#include "fsr3upscaler/ffx_fsr3upscaler_luma_pyramid.h"

[shader("compute")]
[numthreads(256, 1, 1)]
void luma_pyramid_main(uint3 WorkGroupId : SV_GroupID, uint LocalThreadIndex : SV_GroupIndex)
{
    ComputeAutoExposure(WorkGroupId, LocalThreadIndex);
}
