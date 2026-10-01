# #106: the swash's dashed line (2026-10-01)

From above the beach, a grey band ran along the sand where the waves' sheet runs up, with light
dashes along it: from 60 m up it read as a road with its centre line (`coast.md`, "For the owner's
eye").

Each sheet is before (left, b64378f) and now (right), `city-blocks --island 7 --water`:
- `swash-close.png`: 18 m over the south beach (`--view 0,18,5135,0,-60`) at 12 s (top) and 14 s
  (bottom), the top-left of the frame.
- `swash-above.png`: #106's view, 60 m over the south beach (`--view 0,60,5160,0,-60`).
- `swash-beach.png`: along the beach from 8 m (`--view 0,8,5120,70,-14`).

## What it was

Not the coast distance's 8 m samples (smoothing it left the dashes as they were), nor the layer
map's sea floor under the sand (moving its top under the sea's level left them too). Turning the
sea's foam at the swash's edge off took the band and the dashes away: they were that foam.

The sea's shader whitened the sheet where it stood under 8 cm over the sand: a band 2 m wide on
this beach, at 0.7 of the foam's cover. The broken water's pattern (value noise over 2.5 m and
0.9 m) then cut it up, and a band that narrow through blobs that size is a dashed line. At 0.7 ×
the pattern the foam only half covered the wet sand, so the band read grey rather than white.

## What changed

The swash's leading edge is a line of foam that the pattern only frays:
- it shows where the sheet is under 3 cm (`SWASH_EDGE`), a band a few tens of centimetres wide
  on this beach;
- the pattern takes at most 40 % of its cover (`SWASH_EDGE_BREAKUP`), so it never breaks into
  dashes;
- the broken water in the surf keeps the pattern as it was.

## Numbers

**The frame** at 1600 × 900 (three runs each alternating, the waves held at 12 s), before and now:

| View | frame | water/surface |
|---|---|---|
| coast (the first view) | 1.699 → 1.694 ms | 0.154 → 0.150 |
| the island from 2.5 km | 2.151 → 2.157 ms | 0.194 → 0.192 |
| along the beach from 8 m | 1.833 → 1.821 ms | 0.150 → 0.148 |
| the coast east from 70 m | 1.641 → 1.643 ms | 0.113 → 0.112 |
| over the south beach from 60 m | 1.760 → 1.756 ms | 0.194 → 0.190 |
| the round lake | 1.569 → 1.570 ms | 0.086 → 0.086 |

Nothing to pay: the narrower edge runs the pattern's noise on fewer pixels.

**Stability** (pixels changing by more than two levels, still camera, waves held, TAA on), frame
to frame and over 32 frames, before → now:

| View | 1 frame | 32 frames |
|---|---|---|
| coast | 1.324 → 1.345 % | 0.0012 → 0.0010 % |
| the island from 2.5 km | 0.930 → 0.929 % | 0.0010 → 0.0010 % |
| along the beach | 0.249 → 0.252 % | 0.0018 → 0.0017 % |
| the coast east | 0.475 → 0.480 % | 0.0022 → 0.0022 % |
| over the south beach | 0.021 → 0.025 % | 0.0006 → 0.0004 % |
| the round lake | 0.229 → 0.229 % | 0.0011 → 0.0011 % |

## Checks

- **The batch** against b64378f: only the island's water images change (`water60` on both paths,
  3 210 px, ꟻLIP mean 0.0008); everything else at 0 px; the occlusion A/B and mesh against
  fallback at 0 px.
- **Validation:** `tools/validate.sh` clean on both paths (14 runs).
- A shader change only: the build, tests, clippy and fmt are b64378f's.
