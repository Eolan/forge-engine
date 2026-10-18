// FSR 3.1's shading-change pyramid (D-070): AMD's binding numbers of its pass, the bridge, its
// headers. Driven by crates/forge-render/src/fsr.rs.

#define FSR3UPSCALER_BIND_SRV_CURRENT_LUMA 0
#define FSR3UPSCALER_BIND_SRV_PREVIOUS_LUMA 1
#define FSR3UPSCALER_BIND_SRV_DILATED_MOTION_VECTORS 2
#define FSR3UPSCALER_BIND_SRV_INPUT_EXPOSURE 3
#define FSR3UPSCALER_BIND_UAV_SPD_GLOBAL_ATOMIC 0
#define FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_0 1
#define FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_1 2
#define FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_2 3
#define FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_3 4
#define FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_4 5
#define FSR3UPSCALER_BIND_UAV_SPD_MIPS_LEVEL_5 6

#include "fsr_bridge.slang"
#include "fsr3upscaler/ffx_fsr3upscaler_common.h"
#include "fsr3upscaler/ffx_fsr3upscaler_shading_change_pyramid.h"

[shader("compute")]
[numthreads(256, 1, 1)]
void shading_change_pyramid_main(uint3 WorkGroupId : SV_GroupID, uint LocalThreadIndex : SV_GroupIndex)
{
    ComputeShadingChangePyramid(WorkGroupId, LocalThreadIndex);
}
