// FSR 3.1's first pass (D-070): AMD's binding numbers of its pass, the bridge, its headers.
// Driven by crates/forge-render/src/fsr.rs.

#define FSR3UPSCALER_BIND_SRV_INPUT_MOTION_VECTORS 0
#define FSR3UPSCALER_BIND_SRV_INPUT_DEPTH 1
#define FSR3UPSCALER_BIND_SRV_INPUT_COLOR 2
#define FSR3UPSCALER_BIND_UAV_DILATED_MOTION_VECTORS 0
#define FSR3UPSCALER_BIND_UAV_DILATED_DEPTH 1
#define FSR3UPSCALER_BIND_UAV_RECONSTRUCTED_PREV_NEAREST_DEPTH 2
#define FSR3UPSCALER_BIND_UAV_FARTHEST_DEPTH 3
#define FSR3UPSCALER_BIND_UAV_CURRENT_LUMA 4

#include "fsr_bridge.slang"
#include "fsr3upscaler/ffx_fsr3upscaler_common.h"
#include "fsr3upscaler/ffx_fsr3upscaler_prepare_inputs.h"

[shader("compute")]
[numthreads(8, 8, 1)]
void prepare_inputs_main(int2 iDispatchThreadId : SV_DispatchThreadID)
{
    PrepareInputs(iDispatchThreadId);
}
