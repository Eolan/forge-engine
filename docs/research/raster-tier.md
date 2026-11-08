# A tier without rays

What D-108 stands on: sun and lamp shadows on a GPU-driven cluster renderer, screen-space reflections,
and indirect light with no rays at run time. Read on 2026-11-08. Costs come from the pages named;
several talks could not be read, and are listed at the end.

## Sun shadows

- **Cached far cascades.** [CryEngine](https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113/pages/21267738)
  renders cascades past a set one once and keeps them, past 16 km for about 130 MB, drawing dynamic
  objects in the near cascades. [HDRP](https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@16.0/manual/shadow-update-mode.html)
  keeps static casters on the light's update mode, draws dynamic ones every frame, and blits the
  cached map to the dynamic one.
- **Virtual shadow maps.** [Unreal Engine 5](https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-shadow-maps-in-unreal-engine):
  16K virtual maps in 128 × 128 pages, clipmap levels, static and dynamic cached apart. Anything moving
  invalidates the pages it overlaps; a turning light invalidates them all; a full pool corrupts the
  image. An [open implementation](https://graphics-programming.org/blog/virtual-shadow-maps) on an RTX
  3070 with the Bistro: 2.37 ms uncached, 0.39 ms cached; sampling at 1080p 0.31 to 0.89 ms.
- **Stability.** Fit a sphere per split and snap the projection to texels. A partition that moves each
  frame, as [sample distribution shadow maps](https://www.intel.com/content/www/us/en/developer/articles/technical/sample-distribution-shadow-maps.html)
  do, must be quantised or paired with snapped clipmaps. [Reading the range back lags or stalls](https://therealmjp.github.io/posts/shadow-maps);
  a GPU-driven path needs no readback.
- **Clipmaps.** Snap to pages, move only along the light's plane, wrap the addresses, redraw only the
  exposed strips.
- **Clusters.** A LOD cut per view, culling per view, cones from the light's direction, each cascade's
  own previous depth pyramid.

## Lamps and contact

- **An atlas.** [Doom (2016)](https://www.adriancourreges.com/blog/2016/09/09/doom-2016-graphics-study/) keeps
  one 8K atlas with a tile per shadowed light sized by importance, static parts cached, dynamic parts
  composited over them.
- **Contact shadows.** [Bend Studio's screen-space shadows](https://www.bendstudio.com/blog/inside-bend-screen-space-shadows/),
  first in Days Gone. No cost published. The page states no licence, so Forge writes its own march.

## Reflections

[FidelityFX SSSR](https://gpuopen.com/manuals/fidelityfx_sdk/techniques/stochastic-screen-space-reflections/) is
MIT, for D3D12 and Vulkan: hierarchical traversal, a roughness-driven rate, a denoiser, and a prefiltered
cube for rough surfaces and misses. AMD publishes no timings. [Parallax-corrected cubes](https://c0de517e.blogspot.ca/2015/03/being-more-wrong-parallax-corrected.html)
suffer where geometry leaves the proxy, and prefiltering before the correction adds error.

## Indirect light without rays

| Technique | Cost found | Limits |
|---|---|---|
| [Visibility bitmask](https://ar5iv.labs.arxiv.org/html/2301.11376) | RTX 2080, 1080p: GI 2.93 ms full, 1.07 ms half resolution | One layer, guessed thickness, light vanishes when its source leaves the screen |
| [Lumen](https://dev.epicgames.com/documentation/unreal-engine/lumen-performance-guide-for-unreal-engine?lang=en-US) | budgets of 4 and 8 ms on console at 1080p | Scene range, thin walls, moving statics |
| [SDFGI](https://docs.godotengine.org/en/latest/tutorials/3d/global_illumination/using_sdfgi.html) | among the most demanding features, by its docs | Dynamic lights yes, dynamic occluders no |
| Voxel cone tracing | 2 to 3 ms in CryEngine's low-spec form; 27 ms at 720p on 2012 hardware | Leaks, ghosting |

Shipped systems trace the screen first and fall back to world data on a miss. Screen space carries the
last few metres, never the base term; interiors and anything lit from off-screen need baked or
world-space data.

## Checked and left out

Not readable here: the Fortnite virtual shadow map article, the Lumen SIGGRAPH deck, Frostbite's
stochastic SSR page (404), and several SIGGRAPH decks that arrive as binary PDFs. No millisecond figures
were found for SSSR, Brixelizer or Bend's method.

## Recommendation for Forge

Cached, snapped shadow rings for the sun; a short contact march; a shadow atlas for lamps; SSR over
cubes and the sky; D-097's relit probes where a bake exists, with a screen-space near term; and for
worlds that cannot bake, a measurement before any world-space structure. D-108 records what was taken.
