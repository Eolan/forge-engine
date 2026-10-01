# #106: the ground's steps (2026-10-01)

The owner: "on the right-hand side of the default initial map when you launch Island 7, the
terrain is very uneven", and the triangles show "in the hollow thought to be caused by erosion
leading down to the ocean". The valleys' walls step every 8 m, and their shadows cross the
rivers in stripes.

With the sun's shadows off the steps still show: they are the field's, not the shadows'. The 8 m
erosion leaves steps two samples apart on the slopes, one sample a ridge and the next a gully.
One pass of the 3 × 3 binomial filter over the whole field (`GROUND_SMOOTHING`, before the sea
floor) takes out what alternates every sample, halves what repeats every four, and leaves the
valleys, hundreds of metres across, as they were.

Each sheet is the lakes' commit d040765 (left) against now (right: tonight's coast and this),
`city-blocks --island 7 --water`:
- `ground-valley.png`: the largest river's valley from 200 m (`--view=-5400,200,-1416,-90,-30`):
  the walls' stripes are gone, and the river shows in its channel.
- `ground-right.png`: the ground east of the first view (`--view 900,120,4700,-35,-15`).
- `ground-side.png`: the coast east of the first view from 70 m (`--view 450,70,5000,0,-12`):
  the sand's top lost its teeth, which were the same gullies crossing 2.5 m.

## What else it changes

- The rivers stand under their banks by 3.9 m at most, where it was 14.5 m: the gorges the
  level water cut were mostly through those steps (621 points more than 2 m under their banks,
  from about 4 000).
- 14 lakes of a hectare or more where there were 15: the smoothing changes the depressions too.
- The rocks follow the new slopes.

## Numbers

**The frame** at 1600 × 900 (three runs each alternating, the waves held at 12 s), the lakes'
commit against this one (tonight's coast, darker lakes and this together):

| View | frame | shading/layered |
|---|---|---|
| coast (the first view) | 1.691 → 1.687 ms | 0.340 → 0.354 |
| the island from 2.5 km | 2.133 → 2.154 ms | 0.293 → 0.310 |
| the valley from 200 m | 1.797 → 1.804 ms | 0.445 → 0.459 |
| down a river from 3 m | 1.558 → 1.593 ms | 0.371 → 0.410 |
| east of the first view | 1.542 → 1.549 ms | 0.367 → 0.377 |
| the round lake | 1.579 → 1.581 ms | 0.374 → 0.376 |

The smoothing itself costs nothing to draw; the layered shading grows with the coast's wandering
lookup and with the river beds the smoothed valleys show more of.

**Stability** (pixels changing by more than two levels, still camera, waves held, TAA on), the
same commits: over 32 frames the same or better everywhere (the valley 0.0187 → 0.0108 %, down a
river 0.0035 → 0.0005 %); frame to frame within ±0.13 % (the island from 2.5 km 0.80 → 0.92 %,
down a river 0.42 → 0.55 %: more river water in view, its ripples' reflections moving with the
jitter).

## Checks

- **The batch** against the previous commit: only the island's images change (365 405 px, ꟻLIP
  mean 0.045; with the water 769 845 px, 0.077: the whole ground moves a little and the rocks are
  placed on the new slopes); everything else at 0 px; the occlusion A/B and mesh against fallback
  at 0 px.
- **Validation:** `tools/validate.sh` clean on both paths.
- **Tests, clippy, fmt.**
