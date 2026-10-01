# #105: the water's path through the refracted ray (2026-10-01)

The sea near the coast was speckled with yellow points, the sea floor showing through the wave
troughs. They changed from frame to frame: most of the first view's 1.35 % of pixels changing
between frames of a still camera were on the sea.

Each sheet is before (left, dd4e8ac) and now (right), `city-blocks --island 7 --water`, the waves
held at 12 s:
- `refraction-coast.png`: the first view.
- `refraction-shallows.png`: its near sea, closer.
- `refraction-beach.png`: 60 m over the south beach (`--view 0,60,5160,0,-60`).
- `refraction-river.png`: down a river from 10 m (`--view -3062,236.8,-781,0,-10`).

## What it was

The water dims what lies under it by the path the light takes through it. The shader measured
that path along the view ray as if it went straight on: the distance from the surface to the
floor behind the pixel. At a grazing angle that path is ten times the water's depth. A wave 0.3 m
high then changes it by 1.5 to 3 m, and the floor flickers through every trough.

Light crossing into water bends towards the vertical (Snell's law, n = 1.333). Seen at a grazing
angle, the refracted ray still falls at about 48° from the vertical, so its path to the floor is
about 1.5 times the depth.

## What changed

- The sea's, the rivers' and the lakes' path through the water is the depth under the surface
  (from the view ray's meeting with the floor) over the refracted ray's fall (`refracted_down`,
  with the surface's normal), at least a tenth.
- Straight down nothing changes. At grazing angles the shallows are clearer: the floor shows
  through the near sea, a paler green over the sand where it was saturated turquoise, and the
  speckles keep their pattern at a lower contrast. The deep sea stays blue.
- What the water lets through is still the scene straight behind the pixel. Shifting it along
  the refracted ray (Sousa's refraction) belongs to the underwater view (#108).

## For the owner's eye

This changes the look of the shallows, the first view's near sea most of all. It is the physical
path, and the shimmer is lower, but the turquoise before was more saturated. If the paler
shallows read worse, the commit reverts on its own.

## Numbers

**The frame** at 1600 × 900 (three runs each alternating, the waves held at 12 s): the same
within 0.006 ms in six views, `water/surface` within 0.001 ms (0.101 ms at the first view, 0.051
at the round lake).

**Stability** (pixels changing by more than two levels, still camera, waves held, TAA on), frame
to frame and over 32 frames, before → now:

| View | 1 frame | 32 frames |
|---|---|---|
| coast | 1.347 → 1.122 % | 0.0017 → 0.0016 % |
| the island from 2.5 km | 0.928 → 0.927 % | 0.0010 → 0.0006 % |
| along the beach | 0.253 → 0.231 % | 0.0017 → 0.0019 % |
| the coast east | 0.480 → 0.473 % | 0.0022 → 0.0030 % |
| over the south beach | 0.025 → 0.024 % | 0.0004 → 0.0005 % |
| the round lake | 0.229 → 0.228 % | 0.0011 → 0.0013 % |

## Checks

- **The batch** against dd4e8ac: only the island's water images change (`water60` on both
  paths, 492 021 px, ꟻLIP mean 0.062: the near sea); everything else at 0 px; the occlusion A/B
  and mesh against fallback at 0 px.
- **Validation:** `tools/validate.sh` clean on both paths (14 runs).
- A shader change only: the build, tests, clippy and fmt are dd4e8ac's.
