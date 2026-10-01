# #106: the coast's definition (2026-10-01)

The island's contours near sea level (the water's edge, the wet sand's, the sand's top) ran in
straight segments with corners every 8 m, and the owner saw the triangles wherever there is water.
The rivers' and lakes' water was fixed in #105 (channels and shores on cells of a metre); this is
the coast.

Each sheet is before (left, the lakes' commit d040765) and now (right),
`city-blocks --island 7 --water`:
- `coast-above.png`: #106's view, 60 m over the south beach (`--view 0,60,5160,0,-60`).
- `coast-beach.png`: along the beach from 8 m (`--view 0,8,5120,70,-14`).
- `coast-side.png`: the coast east of the first view from 70 m (`--view 450,70,5000,0,-12`).

## What it was

The field itself terraced there, not only its triangles. `sea_floor` sets the samples at sea
from the coast distance and leaves the eroded land at the sea's level, so along a coast that runs
across the grid the samples alternate between land at 0 m and floor at −0.3 m. The sea's level
traced through them steps with the samples, and a cubic through them scallops.

## What changed

- `forge_procgen::smooth_shore`: four passes of a 3 × 3 binomial filter over the samples within
  3.5 m of the sea's level, the rest kept: the coast's contours run smooth through the samples.
- The cells the sea's level or the sand's top (2.5 m) cross, and a cell more, are drawn on cells
  of a metre on the smoothed ground, as the rivers' channels and the lakes' shores are (76 856
  refined cells in all, 6.2 M fine vertices).
- The layered material's lookup wanders by a texel over a noise three texels wide on the island's
  ground (`RenderLayer::cavity` for the layered class, 0 for the city's streets), so the layers'
  edges stop stepping along the 4 m map.

## For the owner's eye

- The sand's top still shows the 4 m layer map's texels, softly and irregularly now.
- A faint dashed line at the swash's edge. (Not the coast distance, as first thought: the foam at
  the swash's edge, cut up by the foam's pattern; fixed since, `swash.md`.)
- Inland the slopes keep their 8 m facets and the ground is as rough as the 8 m erosion left it
  (the right of the first view); that is the 2 m amplification's part of #106.

## Numbers

**The frame** at 1600 × 900 (three runs each alternating, the waves held at 12 s), the lakes'
commit against this one:

| View | frame | shading/layered | water/surface | water/reflections |
|---|---|---|---|---|
| coast (the first view) | 1.691 → 1.709 ms | 0.340 → 0.348 | 0.156 → 0.154 | 0.188 → 0.194 |
| along the beach from 8 m | 1.864 → 1.887 ms | 0.439 → 0.458 | 0.176 → 0.166 | 0.211 → 0.226 |
| the coast east from 70 m | 1.658 → 1.659 ms | 0.418 → 0.429 | 0.121 → 0.118 | 0.081 → 0.080 |
| over the south beach from 60 m | 1.799 → 1.826 ms | 0.488 → 0.509 | 0.219 → 0.213 | 0.190 → 0.189 |
| the island from 2.5 km | 2.132 → 2.152 ms | 0.292 → 0.296 | 0.194 → 0.194 | 0.082 → 0.081 |
| the round lake | 1.577 → 1.601 ms | 0.375 → 0.385 | 0.094 → 0.094 | 0.075 → 0.084 |

The wandering lookup costs up to 0.02 ms of `shading/layered`; the rest is the refined cells.

**Stability** (pixels changing by more than two levels, still camera, waves held, TAA on): along
the beach 0.44 → 0.26 % frame to frame (the stepped wet line flickered), 0.0031 → 0.0015 % over 32
frames; the first view 1.41 → 1.36 % and 0.0025 → 0.0015 %; every other view the same or better.

## Checks

- **The batch** against the lakes' commit: only the island's images change (153 810 px, ꟻLIP mean
  0.029; with the water 555 698 px, 0.063: the shore's floor is smoothed, so the waves feel a
  different shallows and break in a cleaner line); everything else at 0 px; the occlusion A/B and
  mesh against fallback at 0 px.
- **Validation:** `tools/validate.sh` clean on both paths.
- **Tests:** a coast across the grid comes out smooth and the rest stays
  (`forge_procgen::coast`); clippy, fmt.
