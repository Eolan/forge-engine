# FSR 3.1: the version, how it plugs in, and the patents

Research for D-070 (#286), 2026-10-18. FSR 3.1 replaces TAA everywhere, as the anti-aliasing
at native size and as the upscaler off NVIDIA. Frame generation is not used. The patent part
was done by a cloud agent: it read the claims of the patents marked "claims", and the
others from their titles and abstracts. Expiry dates are Google Patents' estimates.
None of this is legal advice.

## The version

- **FSR 3.1.4, AMD FidelityFX SDK v1.1.4** (GitHub `GPUOpen-LibrariesAndSDKs/FidelityFX-SDK`,
  tag `v1.1.4`, MIT). The tag's source archive is 200 MB (SHA-256 `9a19e1f2…cc61179`). It sits in
  the ignored `fidelityfx-sdk/`, downloaded with the owner's yes.
- **The FSR SDK 2.x (2025–2026)** brings FSR 4 and the upscaler 3.1.5, but its readme says
  "Vulkan is currently not supported in SDK". v1.1.4 is the last with a Vulkan backend.

## How it plugs in

- **The upscaler is eight compute passes a frame**, in the order
  `ffx_fsr3upscaler.cpp` schedules them:
  1. prepare inputs;
  2. the luma pyramid (SPD);
  3. the shading-change pyramid;
  4. shading change;
  5. prepare reactivity;
  6. luma instability;
  7. accumulate (with the sharpening built in, or not);
  8. RCAS.
- **Each pass is a header** in `sdk/include/FidelityFX/gpu/fsr3upscaler/`. The header reads and
  writes through about 60 callbacks (`ffx_fsr3upscaler_callbacks_hlsl.h`: `LoadInputColor`,
  `StoreInternalColorAndWeight`, the constant getters, and so on).
- **Forge's bridge** follows RTXDI (D-065):
  - **Shaders:** a Slang file implements those callbacks over Forge's bindless images and a
    frame block read by device address, then includes the SDK's headers from `fidelityfx-sdk/`
    at compile time. One entry point per pass.
  - **Host:** Rust computes the constants as the host code does: jitter, the motion-vector
    scale, the frame index, pre-exposure.
  - **Render graph:** the eight passes are declared there, so they get barriers and profiler
    zones like every other pass.
  - **Not used:** AMD's prebuilt `amd_fidelityfx_vk.dll` and the FFX API. They would record their
    own barriers outside the graph.

## The licence

- **The licence text:** `LICENSE.txt` is plain MIT, and the word "patent" appears nowhere in the
  SDK's own licence or documents.
- **The readme:** it says "No license, including implied or arising by estoppel, to any
  intellectual property rights is granted by this document". That disclaimer is about the
  document, not the MIT grant, but it is no pledge either.
- **AMD:** no patent pledge, non-assert or covenant for FSR or FidelityFX was found
  (https://gpuopen.com/fidelityfx-superresolution-3/).
- **In the US:** one reading holds that MIT's "deal in the Software without restriction" grants
  the licensor's patents (https://opensource.com/article/18/3/patent-grant-mit-license). It is
  untested in court.
- **In France:** CPI L613-8 wants a patent licence in writing
  (https://www.baumann-avocats.com/CPI/article-l613-8-du-code-de-la-propriete-intellectuelle.php).
  No AMD patent on this is in force in France yet.

## AMD's patents

| Patent | Dates, status | Family | Reading |
|---|---|---|---|
| **US 12,293,485 B2** "Super resolution upscaling" (AMD, ATI) | Priority 2022-03-17; granted 2025-05-06; live, to about 2043-11 | WO2023177887; EP4494087 pending; CN119301630A; JP7902272B2 granted; KR | **Claims:** claim 1 upsamples a frame, accumulates it with the previous upscaled frame, and gives history colour held by a pixel lock more weight. The dependent claims add dilated depth and motion, disocclusion, shading change, luma locks, Lanczos, the colour box, reprojection |
| **US 12,493,990 B2** "Locking mechanism for image classification" | Same priority; granted 2025-11; live, to about 2043-12 | WO2023177888; EP23771466.2 (status not read); CN119053986A; JP7864844B2 granted | **Claims:** claim 1 locks pixels with "high-frequency spatial information" and keeps their colour during accumulation |

FSR 3.1.4 still creates locks (`ffx_fsr3upscaler_prepare_reactivity.h`, from
`ComputeThinFeatureConfidence`) and uses them (`ffx_fsr3upscaler_accumulate.h`). Its upsampler is
Lanczos-2. Claim 1 of both patents very likely reads on it. Sources:
https://patents.google.com/patent/US12293485B2/en, https://patents.google.com/patent/US12493990B2/en,
https://patents.google.com/patent/EP4494087A4/en.

**Checked, no match:**
- **US 12,020,408 B2 (claims read):** it needs a hardware blur circuit before the sharpener, and
  RCAS has none.
- **US 10,354,365 B2 (title and abstract):** MSAA with post-process AA.
- **US 11,544,815 B2 (title and abstract):** neural networks.
- **AMD's frame-interpolation applications:** frame generation, which Forge doesn't use.

## Other holders

- **NVIDIA US 10,116,916 B2 and US 10,362,289 B2** (Salvi, Patney, Lefohn; claims read):
  - **Dates:** priority 2016-03-17, US only, live until about 2037 (fees paid 2026-03).
  - **Claims:** '916 builds a box from the neighbourhood's mean ± its standard deviation and
    adjusts the history with it. '289 builds a box from the neighbourhood's statistics, with a
    sample outside it.
  - **What matches:** FSR 3.1.4 does exactly this (`ffx_fsr3upscaler_common.h`, a 1–3σ box in
    `accumulate.h`). So does Forge's TAA today (`shaders/taa.slang`, ±1.25σ).
  - **Sources:** https://patents.google.com/patent/US10116916B2/en,
    https://patents.google.com/patent/US10362289B2/en.
- **Sony, "Temporal supersampling for foveated rendering systems"** (US 10,713,752 B2,
  US 11,087,437 B2, US 11,704,768 B2; priority 2017-06; expiry about 2038):
  - **The applications' claims** cover jittered low-resolution samples gathered across frames
    and reprojected; foveation appears only in dependent claims.
  - **Open:** the granted claims were not read. There is strong prior art: Yang et al.,
    "Amortized supersampling", 2009 (https://www.hhoppe.com/proj/supersample/).
- **NVIDIA US 9,208,605 B1 (claims read):** multisampled, time-offset copies of primitives. No
  match.
- **Intel's XeSS-family applications:** titles only; machine learning or motion-vector work.
- **Litigation:** none found over FSR, TAA or upscaling patents.

## What was chosen (the owner, 2026-10-18)

- **AMD's shaders as published, locks included.** AMD's own MIT code is the closest Forge can
  stay to its grant. Godot ships FSR 2.2 the same way, as native AA
  (https://docs.godotengine.org/en/4.3/tutorials/3d/resolution_scaling.html).
- **The variance box replaced** by a min/max neighbourhood clamp: Karis, "High Quality Temporal
  Supersampling", SIGGRAPH 2014
  (https://advances.realtimerendering.com/s2014/index.html), older than NVIDIA's priority
  date. It computes no variance and has no sample outside its box, so neither claim 1 matches.
  - It is a small patch of AMD's header, kept in Forge.
  - Its effect on ghosting and shimmer is measured against the unpatched shader.
- **The rest is prior art from 2007–2014:** jitter, reprojection, depth-based disocclusion and
  Lanczos (the survey of Yang, Liu and Salvi, 2020:
  https://research.nvidia.com/labs/rtr/publication/yang2020survey/). AMD's patents name these
  only in dependent claims.
- **If the locks must go later:** zero new locks and their contribution. Neither AMD claim 1
  would then match. FSR 3.1's luma instability may take part of their job, to be tested on wires
  and fences.

## Built (2026-10-18)

D-070 has the results: the bridge, the passes, the determinism (0 px run to run and mesh against
fallback) and the measures.
- **Against AMD's variance box,** the min/max box steadies a still camera six times over (0.074 %
  of pixels changing against 0.487 %). It loses a little in a turn: 34.0 % against 33.3 % of
  pixels changing unlike SSAA's, and ꟻLIP 0.0414 against 0.0408.
- **FSR beats DLAA** on all three in the city.
