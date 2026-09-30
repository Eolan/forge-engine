# #105 step 4: the island's shadow on the water (2026-09-30)

With `--water`, the island now shades the sea. The surface asks for a shadow ray per pixel
beside its mirror ray, and the same pass (`water/reflections`) traces both.

- `shadow-sheet.png`: the island from the north-west, 300 m up and 7 km out, against a 10° sun
  (`--sun-elevation 10 --view=-5600,300,-4200,-126.9,-4.6 --sea-time 10`). Left: the previous
  commit, where the sun lights the water right up to the beach. Right: with the shadow.
- `shadow-shore.png`: the same, cropped on the shore (top: before, bottom: after).

## How it works

- **The request:** `water/surface` writes a third target, the share of its colour the sun
  lights, through the same foam and air as the colour.
  - It counts the highlight and the sunlight the water scatters back: under a shaded surface
    the water loses the sun too.
  - The sea floor seen through the water was shaded by its own rays in the resolve, so it
    stays out.
  - A share under 1 % of the pixel's brightest channel asks no ray.
- **The ray:** from the water's point (rebuilt from its depth), towards one of eight points of
  the sun's disc, turned per pixel, as the ground's shadow rays are. TAA averages them into a
  penumbra. A blocked ray takes the share away.
  - It starts 2 m off the water, along the vertical and the sun, the terrain's own start
    (`TERRAIN_SHADOW_START`). The sea floor's traced surface may stand a metre above the drawn
    one, so above the water in the shallows.
- **The key:** **J** turns it off with the other shadows; **F** and **Y** turn off the mirror
  rays alone. The pass runs when either is on.

## Where it shows

- **At the default 30° sun, hardly anywhere.** The island's slopes facing the sea are gentler
  than the sun, so they shade almost none of it: 54 pixels change in the view from the
  north-west, and the coast and sea views don't change at all.
- **Under a low sun, on the side away from it.** At 10° the shadow cuts the sun's glitter path
  short of the shore, with an edge that follows the hills. The shaded water keeps only the
  sky's blue reflection.

## Numbers

**The frame** (3 000 frames, three runs each, alternating, medians), the previous commit
against this one:

| View | previous commit | with the shadow rays | water/reflections |
|---|---|---|---|
| coast, 1600 × 900 | 1.62 ms | 1.65 ms | 0.17 → 0.20 |
| from the sea, 1600 × 900 | 1.42 ms | 1.44 ms | 0.09 → 0.11 |
| north-west into a 10° sun | 1.45 ms | 1.52 ms | 0.07 → 0.14 |
| coast, 2560 × 1440 | 3.10 ms | 3.21 ms | 0.42 → 0.49 |

- **A shadow ray costs less than a mirror ray:** the first hit ends it, and no hit is shaded.
- **Into a low sun** they cost the most: the rays from the water in front of the island cross
  its slopes and rocks on their way to the sun, where elsewhere they leave the scene at once.
- **The third target** (8 bytes a water pixel, cleared each frame) doesn't show in
  `water/surface` (0.073–0.075 ms at the coast either way).

**Stability** (pixels changing by more than two levels, still camera, waves held still,
TAA on):

| View | before, 1 frame | after, 1 frame | before, 32 frames | after, 32 frames |
|---|---|---|---|---|
| north-west, 10° sun | 1.37 % | 1.42 % | 0.021 % | 0.017 % |
| coast | 1.62 % | 1.62 % | 0.0021 % | 0.0021 % |
| from the sea | 0.98 % | 0.98 % | 0.0003 % | 0.0003 % |

The penumbra settles like the ground's: nothing crawls.

## Checks

- **The batch** against step 3's: every image at 0 px, the water's included (the batch's
  water views look at the lit side under the 30° sun), but #71's flake (`fb-ast-taa600`,
  353 px, ꟻLIP mean 0.0014). The occlusion A/B and mesh against fallback are at 0 px.
- **Validation:** `tools/validate.sh` is clean on both paths, synchronization included.
- **Tests, clippy, fmt:** pass.
