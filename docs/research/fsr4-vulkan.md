# FSR 4 on Vulkan (AMD FSR SDK 2.4.0)

*2026-10-28, #361. Follows [fsr-3-1.md](fsr-3-1.md) (D-070). Read from the SDK's own docs and
headers (`Kits/FidelityFX/docs`, `api/include`, `upscalers/include`) at the tag `v2.4.0` of
<https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK>, downloaded on the owner's go to
`D:\sources\workspace-engines\fsr-sdk-2.4.0` beside the repository. Built on the owner's go
(2026-10-29) as D-082: "Built", at the end.*

## What changed

AMD's SDK 2.4.0 (2026-10-28) brings **FSR Upscaling 4.1** (the ML upscaler) to **Vulkan**, "at
feature parity with the DX12 version". Until then FSR 4 was DX12 only, which is why D-070 made our
own Slang port of FSR 3.1 the anti-aliasing wherever DLSS does not run. The same release brings
FSR 3.1.6 and FSR frame generation 4.0 to Vulkan.

| | |
|---|---|
| GPUs for FSR 4 | RX 9000 (FP8 model), RX 7000 and RX 6000 (INT8), some Ryzen AI APUs (a lighter INT8 model). Not our dev machine's RDNA 2 iGPU, not NVIDIA |
| Cost (AMD's table, performance mode, RCAS off) | RX 9070 XT: 0.35 ms at 1080p, 1.32 ms at 4K. RX 6600: 2.18 ms at 1080p |
| Memory | working set 81 MB at 1080p, 318 MB at 4K |
| Shipping | signed DLLs only: `amd_fidelityfx_loader_vk.dll` (39 KB) loads `amd_fidelityfx_upscaler_vk.dll` (36 MB) from beside itself or the executable |
| Licence | every file of the SDK is MIT, the signed DLLs included (`Kits/FidelityFX/docs/license.md` lists them). MIT has no explicit patent grant: AMD's own patents on its ML upscaling are to be looked up before a release relies on it (the patent rule); the DLLs are never published, only used locally (the owner, 2026-10-29) |

## How it would fit

FSR 4 cannot be ported the way we ported FSR 3.1: the model and its kernels are in AMD's DLL.
The DLL records its own work into our command buffer, the way DLSS does through Streamline
(`forge-gpu/src/dlss.rs`):

- **The API** (`ffx_api.h`, `ffx_upscale.h`, `vk/ffx_api_vk.h`): `ffxCreateContext` with an
  `ffxCreateContextDescUpscale` (max render and output sizes, flags), chained to an
  `ffxCreateBackendVKDesc` holding `VkDevice`, `VkPhysicalDevice` and `vkGetDeviceProcAddr`, and
  to `ffxCreateContextDescUpscaleVersion` (required since SDK 2.1). Then each frame
  `ffxDispatch` with an `ffxDispatchDescUpscale`: our command buffer, the images (color, depth,
  motion, exposure, output), jitter, motion-vector scale, frame time, reset. `ffxQuery` lists the
  versions the driver offers (FSR 4.x where the GPU runs it, 3.1.x otherwise) and the memory.
- **Inputs we already make for FSR 3.1:** linear color at render size, depth (reversed and
  infinite: `FFX_UPSCALE_ENABLE_DEPTH_INVERTED | _INFINITE`, our projection), motion vectors
  without jitter, exposure (1×1) or its auto-exposure flag. FSR 4 no longer needs the reactive
  or the transparency-and-composition mask (they may still be given).
- **The render graph** declares the images the dispatch reads and writes, as for DLSS; the DLL
  manages its own barriers inside, so the pass leaves the images in the states it was given.
- **Selection:** on an AMD device where `ffxQuery` offers FSR 4, `r.aa=fsr` takes it; elsewhere
  our FSR 3.1 port stays (D-070). The DLL is loaded only on AMD (the cross-vendor rule: vendor
  SDKs optional, loaded only for their vendor).

## Open questions

1. **Device extensions.** The Vulkan backend's header asks for nothing but the device, but AMD's
   sample framework (`Kits/Cauldron2/framework/render/vk/device_vk.cpp`) enables, where present,
   `VK_KHR_cooperative_matrix`, `VK_EXT_shader_float8` and `VK_KHR_shader_integer_dot_product`:
   presumably the FP8 model's and the INT8 models' needs. `VK_EXT_shader_float8` is newer than
   ash 0.38's headers (1.3.281); ash's master, which Forge takes since 7e26e1ed, has it. Which of
   them FSR 4 strictly needs is to confirm with the validation layer on an AMD card.
2. **Testing.** No RX 6000/7000/9000 here. On the 5070 Ti we can check that the loader loads,
   that `ffxQuery` answers (presumably FSR 3.1.x only) and that a failure falls back to our port.
   The RDNA 2 iGPU (`FORGE_GPU=amd`) tells whether AMD's driver offers FSR 4 there (not on AMD's
   list).
3. **FSR 3.1.6 for our port:** its shader headers differ from our 3.1.4 source by one line (the
   history sampled at the previous frame's output size, `ffx_fsr3upscaler_reproject.h`), which
   matters only under dynamic resolution. Its "increased precision" is not in the headers.
4. **Patents:** AMD's patents on ML upscaling and their licence terms for the binary's users.

## Proposal (decided: D-082)

A D-entry when the owner asks for it: FSR 4 behind `r.aa=fsr` on AMD, through the signed DLLs,
the Slang FSR 3.1 port kept as the fallback everywhere else, built once there is a card to test
it on (#67) or as a load-and-fall-back path before that.

## Built (2026-10-29, D-082)

`forge_gpu::ffx` (the binding, the `ffx` feature), `forge_render::FfxUpscaler` (the graph pass
"temporal/FSR (AMD)") and `--fsr-amd auto|on|off` under `r.aa=fsr`. What the build answered:
- **The version names** are numbers ("3.1.6", "2.3.4"; a driver's "FSR 4.1.3 *"), not
  "FSR 3.1.6": versions are compared by number. The DLLs' own choice (the newest) is kept where
  it is FSR 4, since an override opts the context out of the drivers' updates; one is given
  only where an FSR 4 is offered and is not their choice.
- **Open question 1, in part:** the DLLs' FSR 3.1.6 declares
  `SPV_KHR_compute_shader_derivatives` (`ComputeDerivativeGroupLinearKHR`), which Forge now enables
  where present, with what AMD's framework enables for FSR 4 (16-bit storage, integer dot
  products, cooperative matrices, 8-bit floats). Whether FSR 4 needs more waits for an AMD card.
- **Open question 2:** on the 5070 Ti and on the RDNA 2 iGPU the DLLs offer 3.1.6 and 2.3.4 only.
  Their 3.1.6 runs on both (0.288 ms and 14.2 ms, against Forge's 0.317 and 18.7 with RCAS); the
  iGPU's `auto` kept Forge's FSR 3.1 until the owner's word the same morning; since then `auto`
  takes the DLLs on any GPU wherever the system has them (FSR 4 where offered).
- **The validation layer** warns, inside the DLLs, of 16-bit storage images written through
  32-bit format declarations (`rw_spd_mip*`, `rw_current_luma`): AMD's own resources.
