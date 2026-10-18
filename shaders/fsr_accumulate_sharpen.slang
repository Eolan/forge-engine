// FSR 3.1's accumulation (D-070) with AMD's sharpening option: it keeps its colour for RCAS
// (`fsr_rcas.slang`), which writes the output.

#define FFX_FSR3UPSCALER_OPTION_APPLY_SHARPENING 1
#include "fsr_accumulate.slang"
