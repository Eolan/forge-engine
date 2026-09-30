# #105 step 2: the sea's surface, first look (2026-09-30)

`city-blocks --island 7 --water` draws the island's sea from the GPU's FFT cascades. It is
behind the flag until the owner has judged it; the default keeps the stand-in, a flat mirror
at 0 m.

`sheet.png` shows the coast view and the view from the sea (`--view 0,300,6800,0,-4.6`),
stand-in on the left and water on the right, with the waves held at 10 s (`--sea-time 10`).
`coast-water.png` and `sea-water.png` are the right-hand frames at full size.

## What is drawn

- **The mesh:** a clipmap of 13 levels of 128 × 128 quads around the camera, the finest
  0.5 m apart (64 m across), the coarsest 262 km across. Each level is centred on the camera
  snapped to twice its spacing and leaves out what the finer level covers. Near its edge a
  level's odd vertices slide onto the coarser lattice, all the way by the edge, so the levels
  meet without cracks and nothing pops as the camera moves.
- **The waves:** the three cascades displace the vertices, each read at the mip matching the
  vertex spacing.
- **The normal:** the fragment takes it from the slopes' mips, with the variance of the slopes
  they no longer resolve as GGX roughness (Bruneton, Neyret & Holzschuch 2010). Far away the
  surface turns rough rather than shimmering.
- **The shading:**
  - Schlick's Fresnel (F0 0.02) with the sky-view table in the mirror direction, blurred
    towards the sky's irradiance as the surface roughens;
  - the sun's GGX highlight;
  - what lies under the water: the scene copied before the pass, dimmed along the view
    ray's path through the water (absorption 0.35, 0.07, 0.05 m⁻¹), plus the light the
    water scatters back;
  - foam where the Jacobian drops under 0.45;
  - the aerial perspective, as the compose applies it to everything else.
- **The graph:** `water/scene-copy`, then `water/surface` depth-tested against the opaque
  depth and writing depth, so TAA's motion vectors and the harness see the sea as a surface.

## What it lacks, for the owner's judgement

- **The island's reflection.** The stand-in mirrors the island through the traced mirror
  rays; the water reflects only the sky. The next step is to trace the water's mirror rays
  (the stand-in's pass), or screen-space reflections.
- **The sun's shadow on the water, and the shore's waves and foam line** (D-038's shore step).
- **Colour:** the shallows over the sand are turquoise, the open sea a deep blue. The
  absorption and the scattered light are the two levers.

## Numbers

**The frame** (3 000 frames, three runs each, medians):

| View | stand-in | water |
|---|---|---|
| coast, 1600 × 900 | 1.26 ms | 1.43 ms |
| from the sea, 1600 × 900 | 1.11 ms | 1.32 ms |
| coast, 2560 × 1440 | 2.38 ms | 2.66 ms |

- **The coast's passes:** `water/surface` 0.07 ms (0.12 ms at 1440p), `water/scene-copy`
  0.01 ms.
- **The compute chain:** FFT columns 0.07, rows 0.06, evolve 0.04, mips 0.03, derive
  0.02 ms.
- **What goes:** the stand-in's traced reflections (0.13 ms), and its share of the standard
  resolve.

**Stability:** pixels changing by more than two levels, still camera, waves held still, TAA
on:

| View | stand-in, 1 frame | water, 1 frame | stand-in, 32 frames | water, 32 frames |
|---|---|---|---|---|
| coast | 0.37 % | 0.43 % | 0.0059 % | 0.0016 % |
| from the sea | 0.34 % | 0.57 % | 0.0003 % | 0.0003 % |

The frame-to-frame change is the jitter's, a little more from the sea (the glitter). After
32 frames the water is as settled as the stand-in: nothing crawls.

## Checks

- **The batch:** 38 images (the water's four added, at a fixed step). The water with
  occlusion off and the mesh path against the fallback are at 0 px. Every other image is
  unchanged but #71's flake.
- **Validation:** `tools/validate.sh` gains the water runs, and synchronization validation is
  clean on both paths.
  - The first run was not: after a vertex shader read every mip, the next frame's mip passes
    on the compute queue took their barriers from graphics stages. The render graph now marks
    the mips a pass crossing queues leaves alone as last used on the other queue (a test in
    `forge-gpu`).
- **The cascades' start-up check** still passes (2.9 × 10⁻⁶).
