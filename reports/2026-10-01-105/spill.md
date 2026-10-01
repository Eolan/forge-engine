# #105: a river's white water spills into the sea (2026-10-01)

The largest river reaches the sea in a cascade (its last 160 m fall at 13 to 17 %, the terrain's
grading, #106), white all the way down. The white water stopped where the river's ribbon fades
out (its last 0.3 m of level, one 4 m segment): from above, the gap through the beach showed the
sea's plain water between the cascade and the surf.

Each sheet is before (left, 9a366be) and now (right), `city-blocks --island 7 --water`, the waves
held at 12 s:
- `spill-above.png`: the largest mouth from 60 m (`--view -5075,60,-1385,42.4,-70`).
- `spill-mouth.png`: 30 m up the river from its mouth (`into_sea` in the log,
  `--view -5083,6.9,-1393,42.4,-20`).
- `spill-low.png`: 2 m over the water near the mouth (`--view -5100,2.2,-1405,40.6,-6`).

## What changed

- Each mouth carries how much of its river runs white there, 0..1: the rapids' rule of the
  river's shader (its fall from 6 to 20 %, its speed from 1.5 to 3 m/s) at its strongest over
  the last 16 m.
- The sea's estuary takes it up: full up the channel, fading over six half widths out along the
  plume and over its edge. It whitens the sea through the sea's own foam pattern, drifting out
  with the river's flow.

## For the owner's eye

- Close up, the spilled white water is the sea's coarser foam pattern (2.5 m cells) beside the
  river's streaks (0.35 m across).
- The line across the water where the ribbon ends, seen from 2 m over the mouth, is still there
  under the foam. Handing the river over by the floor's height per pixel removed it, but showed
  where the river's white water stopped in a dark oval (#105's thread). This change is the
  other half of that handover.

## Numbers

**The frame** at 1600 × 900 (three runs each alternating, the waves held at 12 s), before → now:

| View | frame | water/surface |
|---|---|---|
| the stone in the fastest water | 1.808 → 1.814 ms | 0.122 → 0.125 |
| the largest mouth | 1.898 → 1.903 ms | 0.127 → 0.131 |
| 2 m over the mouth | 1.695 → 1.702 ms | 0.153 → 0.161 |
| coast (the first view) | 1.641 → 1.641 ms | 0.101 → 0.104 |
| the island from 2.5 km | 2.122 → 2.126 ms | 0.144 → 0.148 |
| over the south beach from 60 m | 1.723 → 1.725 ms | 0.146 → 0.149 |

**Stability** (pixels changing by more than two levels, still camera, waves held, TAA on), frame
to frame: the stone 0.944 → 0.932 %, the mouth 0.833 → 0.827 %, 2 m over the mouth 0.721 →
0.668 %, the others unchanged; over 32 frames the same.

## Checks

- **The batch** against 9a366be: only the island's water images change (`water60`, 144 px, a
  mouth far off; ꟻLIP mean 0.00003); everything else at 0 px; the occlusion A/B and mesh against
  fallback at 0 px.
- **Validation:** `tools/validate.sh` clean on both paths (14 runs).
- **Tests**, clippy, fmt.
