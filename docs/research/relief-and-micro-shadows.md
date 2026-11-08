# Relief without geometry and micro-shadows

What D-107 stands on: per-pixel height fields in a compute visibility-buffer resolve, their own
shadows, and the micro-shadows of occlusion and normal maps. Read on 2026-11-08. Where a formula
comes from code that cites a paper rather than from the paper, it says so.

## The march

- **Linear search, then the secant.** Unity's HDRP ships a fixed step count from its caller, a
  linear search and up to three secant iterations
  ([PerPixelDisplacement.hlsl](https://raw.githubusercontent.com/Unity-Technologies/Graphics/master/Packages/com.unity.render-pipelines.core/ShaderLibrary/PerPixelDisplacement.hlsl)).
  It needs no bake and works with blended layers.
- **Steps by the view's angle.** HDRP takes `lerp(minSamples, maxSamples, unitAngle)`.
- **Relaxed cones** ([GPU Gems 3](https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-18-relaxed-cone-stepping-relief-mapping)):
  15 cone steps and 6 binary ones, and no thin features missed, but an offline bake per map, no
  ordinary mips, and a cone map for one height does not combine with blended layers (our inference).
- **Maximum mips and quadtrees** (Tevs 2008, Drobot 2009): a hierarchy per map and a traversal that
  must survive tiling; only secondary numbers were reached.
- **Gradients.** Colour and normal look-ups use the derivatives of the unperturbed coordinates,
  which avoids seams at occlusion edges
  ([GPU Gems 2](https://developer.nvidia.com/gpugems/gpugems2/part-i-geometric-complexity/chapter-8-pixel-displacement-mapping-distance-functions)).
  In a `SampleGrad` resolve: one level for the march from the analytic gradients, then the final
  samples with those gradients at the moved coordinates.
- **Cost.** One modern figure was found: a full-screen height-field march on an RTX 5070 at
  1280 × 800 takes 0.24 ms at 16 steps ([arXiv 2608.17390](https://arxiv.org/html/2608.17390)).
  Scaled by pixels to 1707 × 960 that is about 0.4 ms. That is an extrapolation.
- **"FPOM"** is a product name (Fast Parallax Occlusion Mapping, a UE5 asset by Tore Lervik), not a
  term from the literature.

## Distance and motion

- HDRP fades the offset by mip: `offset *= 1 - saturate(lod - lodThreshold)`.
- No published study of parallax under DLSS was found. Forge's reasoning: the march is a
  deterministic function of the surface point, so jitter only moves the sampling point; shimmer
  then comes from an under-filtered height, so the level must follow the footprint. Noise in the
  step counts would lean on history to clean up, against D-070's reference image.
- **Depth.** HDRP adds its depth offset to clip-space w of both the current and the previous
  position, so depth and motion agree. Epic's pixel depth offset needs code at raster time;
  [Nanite took a programmable rasteriser](https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-5.1-release-notes)
  for it. A compute resolve cannot change hardware depth.

## Shadows from the relief

No published recipe was found for joining a height-field shadow to a ray-traced macro shadow.
Tatarchuk's soft-shadow formula could not be read. Forge's design: an occlusion march up the ray
from the hit, softened by how far the ground stands inside the ray, multiplied into the macro
shadow, and let go towards the macro terminator.

## Micro-shadows

- **Occlusion maps on direct light** (Brinck and Maximov, "The Technical Art of Uncharted 4", 2016,
  through HDRP's `ComputeMicroShadowing`, which cites the talk): `saturate(N·L + 2·AO² − 1)`. The
  slides could not be opened; a forum copy has |N·L|, which agrees wherever N·L is positive.
- **Chan 2018** (through a code copy; XeGTAO's README confirms the use):
  `saturate(N·L / sqrt(1 − visibility))²`. To compare with the first.
- **Against darkening twice.** [Filament](https://raw.githubusercontent.com/google/filament/main/shaders/src/surface_ambient_occlusion.fs)
  and HDRP take the minimum of the material's occlusion and the screen's on indirect light. HDRP's
  direct light never takes the material's.
- **The bump terminator.** Chiang, Li and Burley 2019, through
  [Yining Li's implementation](https://blog.yiningkarlli.com/2020/02/shadow-terminator-in-takua.html):
  `G = min(1, ⟨g,l⟩ / (⟨s,l⟩⟨g,s⟩))`, then `−G³ + G² + G`. Conty Estevez, Lecocq and Stein 2019,
  through Mitsuba 3 and Cycles: a Smith term of the angle between the two normals, on diffuse only.

## Height from a normal map

Frankot and Chellappa's FFT integration suits periodic tiling textures and loses the mean height
([survey](https://ar5iv.labs.arxiv.org/html/1709.05940)). Shipped tools expose intensity and
auto-levels; none documents an absolute scale, and
[Adobe says the result cannot be fully correct](https://experienceleague.adobe.com/en/docs/substance-3d-designer/using/substance-graphs/nodes-reference-for-substance-graphs/node-library/filters/normal-map/normal-to-height-hq).
A cook would store the range and take a scale per material.

## Checked and left out

The original texts of Tatarchuk, Drobot, Tevs, Jimenez, the Uncharted 4 slides, the Chiang talk and
the Conty chapter were not read. Their formulas here come from the code named above.

## Recommendation for Forge

Linear search and secant, deterministic, a level per pixel from the analytic gradients, inward only,
the offset faded by the level. An occlusion march for the relief's shadow. Micro-shadows from the
material's own occlusion on direct light, and the minimum with GTAO on indirect. D-107 records what
was taken.
