// FSR 3.1's reactivity and locks (D-070): AMD's binding numbers of its pass, the bridge, its
// headers. Driven by crates/forge-render/src/fsr.rs.

#define FSR3UPSCALER_BIND_SRV_RECONSTRUCTED_PREV_NEAREST_DEPTH 0
#define FSR3UPSCALER_BIND_SRV_DILATED_MOTION_VECTORS 1
#define FSR3UPSCALER_BIND_SRV_DILATED_DEPTH 2
#define FSR3UPSCALER_BIND_SRV_REACTIVE_MASK 3
#define FSR3UPSCALER_BIND_SRV_TRANSPARENCY_AND_COMPOSITION_MASK 4
#define FSR3UPSCALER_BIND_SRV_ACCUMULATION 5
#define FSR3UPSCALER_BIND_SRV_SHADING_CHANGE 6
#define FSR3UPSCALER_BIND_SRV_CURRENT_LUMA 7
#define FSR3UPSCALER_BIND_SRV_INPUT_EXPOSURE 8
#define FSR3UPSCALER_BIND_UAV_DILATED_REACTIVE_MASKS 0
#define FSR3UPSCALER_BIND_UAV_NEW_LOCKS 1
#define FSR3UPSCALER_BIND_UAV_ACCUMULATION 2

#include "fsr_bridge.slang"
#include "fsr3upscaler/ffx_fsr3upscaler_common.h"
#include "fsr3upscaler/ffx_fsr3upscaler_prepare_reactivity.h"

[shader("compute")]
[numthreads(8, 8, 1)]
void prepare_reactivity_main(int2 iDispatchThreadId : SV_DispatchThreadID)
{
    PrepareReactivity(iDispatchThreadId);
}
