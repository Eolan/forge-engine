# #105 step 3: the island in the water (2026-09-30)

With `--water`, the sea now reflects the island. The surface asks, per pixel, for the mirror
ray it would trace, and a compute pass traces it against the scene, as the glass's rays are
(#50, #52).

- `reflection-sheet.png`: the coast view and the view from the sea. Left to right: the
  stand-in plane, the water with the sky alone (step 2), and the water with the island traced.
  The waves are held at 10 s (`--sea-time 10`).
- `reflection-shore.png`: the same, cropped on the shore.

## How it works

- **The surface** (`water/surface`) writes a second target beside its colour, the request:
  the mirror direction (the same one it read the sky in) and the weight of what a ray meets
  there instead of that sky. The weight is the Fresnel term, less the blur towards the sky's
  irradiance on a rough surface, the foam and the air in front. The target starts at zero
  each frame, so a pixel without water asks nothing.
- **The rays** (`water/reflections`, `MeshletRenderer::trace_requested`): one thread a pixel.
  It rebuilds the water's point from the depth the surface wrote, traces the ray against the
  TLAS, and on a hit adds weight × (hit − sky). The hit is lit as the glass's are: the sun
  through a shadow ray, and the probes' light.
- **The keys:** **Y** (`--no-ray-reflections`) and **F** turn the rays off, as for the glass;
  a GPU without ray queries keeps the sky.

## What it shows

- **From the sea:** a darker, greener band under the island, broken by the waves. The stand-in
  showed a sharp mirror image.
- **At the coast:** the wave faces turned towards the camera, which reflected the bright sky
  low over the horizon, now show the grass slopes and the beach behind the shore, in olive and
  sand patches. The faces turned away still show the sky.
- **Hits on the sea floor:** checked with a false colour of each ray's hit distance. The rays
  from the water near the camera reach the slopes more than 100 m away, and next to none hit
  anything under 0.5 m above the sea. The sea floor's traced surface doesn't show through.

## Numbers

**The frame** (3 000 frames, three runs each, alternating, medians): the previous commit, this
one with the rays off (**Y**), and with them.

| View | previous commit | rays off | rays on | water/reflections |
|---|---|---|---|---|
| coast, 1600 × 900 | 1.47 ms | 1.56 ms | 1.64 ms | 0.17 |
| from the sea, 1600 × 900 | 1.44 ms | 1.40 ms | 1.44 ms | 0.09 |
| coast, 2560 × 1440 | | 2.76 ms | 3.19 ms | 0.43 |

- **The request target** costs `water/surface` at most 0.003 ms. The frames with the rays off
  fall on both sides of the previous commit's: these views move by about ±0.07 ms from run to
  run, every zone together (the GPU's clock).
- **The rays** cost their zone, which grows with the water's pixels. The stand-in's took
  0.13 ms at the coast: a flat mirror's rays stay coherent, and the waves scatter these.
- **Step 2's numbers** (1.43 and 1.32 ms) came from a quieter GPU: the previous commit measures
  1.47 and 1.44 ms today, while the stand-in measures as it did.
- **Levers, if the cost matters:** trace at half resolution and let TAA fill in, or keep the
  300 000 rocks out of the water's rays with an instance mask.

**Stability:** pixels changing by more than two levels, still camera, waves held still, TAA
on.

| View | sky alone, 1 frame | island traced, 1 frame | sky alone, 32 frames | island traced, 32 frames |
|---|---|---|---|---|
| coast | 0.43 % | 1.62 % | 0.0016 % | 0.0021 % |
| from the sea | 0.57 % | 0.98 % | 0.0003 % | 0.0003 % |

- **The frame-to-frame change:** isolated pixels on the edges of the reflected slopes, where a
  wave face's ray flips between the island and the sky as the jitter moves. At the coast,
  0.21 % change by more than 8 levels (0.011 % with the sky alone).
- **After 32 frames** it is as settled as before: nothing crawls. Whether the edges read as
  glitter or as shimmer in motion is the owner's call.

## Checks

- **The batch** against step 2's: only the four water images change (`mesh-water60`,
  `fb-water60` and their occlusion-off twins: 237 534 px, ꟻLIP mean 0.052, max 0.63). Every
  other image is at 0 px, #71's flake included this time. The occlusion A/B and mesh against
  fallback are at 0 px, the water's included.
- **Validation:** `tools/validate.sh` is clean on both paths, synchronization included; the
  water runs trace the rays.
- **Tests, clippy, fmt:** pass.
