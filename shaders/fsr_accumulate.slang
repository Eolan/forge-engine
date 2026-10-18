// FSR 3.1's accumulation (D-070): AMD's binding numbers of its pass, the bridge, its headers.
// Driven by crates/forge-render/src/fsr.rs. `fsr_accumulate_sharpen.slang` compiles it again
// with AMD's sharpening option: the output is then RCAS's (`fsr_rcas.slang`).

#define FSR3UPSCALER_BIND_SRV_INPUT_EXPOSURE 0
#define FSR3UPSCALER_BIND_SRV_DILATED_REACTIVE_MASKS 1
#define FSR3UPSCALER_BIND_SRV_DILATED_MOTION_VECTORS 2
#define FSR3UPSCALER_BIND_SRV_INTERNAL_UPSCALED 3
#define FSR3UPSCALER_BIND_SRV_FARTHEST_DEPTH_MIP1 5
#define FSR3UPSCALER_BIND_SRV_CURRENT_LUMA 6
#define FSR3UPSCALER_BIND_SRV_LUMA_INSTABILITY 7
#define FSR3UPSCALER_BIND_SRV_INPUT_COLOR 8
#define FSR3UPSCALER_BIND_UAV_INTERNAL_UPSCALED 0
#define FSR3UPSCALER_BIND_UAV_UPSCALED_OUTPUT 1
#define FSR3UPSCALER_BIND_UAV_NEW_LOCKS 2

#include "fsr_bridge.slang"
#include "fsr3upscaler/ffx_fsr3upscaler_common.h"
#include "fsr3upscaler/ffx_fsr3upscaler_sample.h"
#include "fsr3upscaler/ffx_fsr3upscaler_upsample.h"
#include "fsr3upscaler/ffx_fsr3upscaler_reproject.h"
#include "fsr3upscaler/ffx_fsr3upscaler_accumulate.h"

[shader("compute")]
[numthreads(8, 8, 1)]
void accumulate_main(int2 iDispatchThreadId : SV_DispatchThreadID)
{
    Accumulate(iDispatchThreadId);
}
