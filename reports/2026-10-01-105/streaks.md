# #105: the rivers' white water in streaks down the river (2026-10-01)

Seen while reviewing the logged views for the owner:
- At the largest river's mouth the white water covered the channel in soft round blotches.
- Behind the stones it ran in smooth white bands metres long.
- Before one stone it bent into a swirl.

Each sheet is before (left, 2bcb6d8) and now (right), `city-blocks --island 7 --water`, the
waves held at 12 s:
- `streaks-mouth.png`: the largest river 30 m before the sea (`into_sea` in the log,
  `--view -5083,6.9,-1393,42.4,-20`).
- `streaks-stone.png`: the stone in the fastest water (`stone_view`,
  `--view -5091,4.6,-1395,40.6,-14`).
- `streaks-sea.png`: down a river to the sea (`--view -3131,46.1,3911,133.1,-10`).
- `streaks-gorge.png`: a gorge with a stone (`--view 290,202.2,4020,-178.5,-10`).

## What it was

The white water's pattern rode the ripples' flow map: two samples carried along the flow, each
restarted every 2 s. Around a stone the flow bends and speeds up. Over a period the pattern
travels 6 m at 3 m/s and is drawn out by the flow's shear: into bands behind the stone, into a
swirl before it. Where the flow runs straight, the round noise cells stayed round blotches.

Streaking the noise along the local flow in world space is not stable: the slightest turn of
the flow rotates coordinates thousands of metres from the origin. (An earlier attempt showed as
lines across the flow from low down, and isotropic cells were kept.)

## What changed

- The white water's pattern is laid out in the ribbon's own coordinates: metres along the
  river from its head (each point's distance, uploaded in the point's last free slot), and
  metres across it.
- Its cells are 0.35 m across and four times that down the river, two octaves.
- It is carried down the river at the river's own speed, in the same two phases as before. The
  flow around a stone no longer draws it out; the stones still decide where the white water is
  (the pillow before them, the wake behind).
- The ripples keep following the flow around the stones.

## For the owner's eye

A change of look: rapids and wakes now streak down the river. From low over the water the
streaks near the camera are long in perspective.

## Numbers

**The frame** at 1600 × 900 (three runs each alternating, the waves held at 12 s), before → now:

| View | frame | water/surface |
|---|---|---|
| the stone in the fastest water | 1.810 → 1.814 ms | 0.120 → 0.122 |
| the largest mouth | 1.899 → 1.904 ms | 0.125 → 0.128 |
| a gorge with a stone | 1.704 → 1.704 ms | 0.041 → 0.042 |
| down a river to the sea | 1.585 → 1.585 ms | 0.054 → 0.054 |

**Stability** (pixels changing by more than two levels, still camera, waves held, TAA on), frame
to frame and over 32 frames, before → now:

| View | 1 frame | 32 frames |
|---|---|---|
| the stone | 0.941 → 0.944 % | 0.0019 → 0.0017 % |
| the mouth | 0.836 → 0.833 % | 0.0020 → 0.0019 % |
| the gorge | 0.162 → 0.165 % | 0.0011 → 0.0006 % |
| down to the sea | 0.185 → 0.180 % | 0.0005 → 0.0006 % |

From 2 m over the mouth's white water the streaks run down the river, with no lines across it.

## Checks

- **The batch** against 2bcb6d8: every image at 0 px (its views have no white water); the
  occlusion A/B and mesh against fallback at 0 px.
- **Validation:** `tools/validate.sh` clean on both paths (14 runs).
- **Tests**, clippy, fmt.
