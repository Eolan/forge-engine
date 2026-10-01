# #106: the sand's top (2026-10-01)

The sand and the grass met in teeth along the island's coast, a few metres deep and about ten
apart (`coast.md`, "For the owner's eye": "the sand's top still shows the 4 m layer map's
texels").

Each sheet is before (left, 3c651fd) and now (right), `city-blocks --island 7 --water`:
- `sand-above.png`: #106's view, 60 m over the south beach (`--view 0,60,5160,0,-60`).
- `sand-beach.png`: along the beach from 8 m (`--view 0,8,5120,70,-14`).
- `sand-side.png`: the coast east of the first view from 70 m (`--view 450,70,5000,0,-12`).

## What it was

The layered pass shades the two layers weighing most among the four texels of the layer map
around the pixel, 4 m apart on the island. Since #106's first step the lookup wanders by a texel
over a noise three texels wide, so the edges stop following the map's grid; along the sand's top
that turned the 4 m steps into teeth.

## What changed

The map's rule for the sand is a height: gentle ground under 2.5 m. The drawn ground there is on
cells of a metre (the coast's contours are refined), so the layered pass applies the rule under
each pixel (`RenderLayer::contour`, `LayerContour` in `forge-core`):
- Where the map shows the sand or one of the grasses (grass, dry, lush), the pixel takes the sand
  under 2.5 m and a grass over it: the map's own grass, or where the map shows sand, the heaviest
  grass among the four texels (none: the sand stays).
- The height wanders by 0.3 m over two octaves of noise 7 m and 2.3 m wide, so the edge bends
  along the beach. The layers blend over 8 cm of height, or the height the pixel spans.
- The other layers keep the map's edges (rock, the sea floor, the rivers' and lakes' beds); where
  one of them meets both the sand and a grass, three layers are shaded.
- The traced rays' hits take the sand under the height too.
- The material row grows from 96 to 112 bytes (the contour's layer, its layers over the height
  as a mask, the height and the wander).

## Numbers

**The frame** at 1600 × 900 (three runs each alternating, the waves held at 12 s), 3c651fd
against this one:

| View | frame | shading/layered |
|---|---|---|
| coast (the first view) | 1.691 → 1.710 ms | 0.356 → 0.374 |
| the island from 2.5 km | 2.143 → 2.174 ms | 0.309 → 0.332 |
| along the beach from 8 m | 1.829 → 1.843 ms | 0.468 → 0.487 |
| the coast east from 70 m | 1.641 → 1.657 ms | 0.434 → 0.450 |
| over the south beach from 60 m | 1.756 → 1.770 ms | 0.498 → 0.508 |
| the round lake | 1.573 → 1.582 ms | 0.374 → 0.388 |

The contour costs 0.010 to 0.023 ms of `shading/layered`. The first version paid for the noise
on every pixel (up to 0.033 ms); the noise now runs only within the wander and the band of the
height, and a pixel over them with no sand around skips the contour. The images are the same to
the bit.

**Stability** (pixels changing by more than two levels, still camera, waves held, TAA on), frame
to frame and over 32 frames, before → now:

| View | 1 frame | 32 frames |
|---|---|---|
| coast | 1.345 → 1.347 % | 0.0010 → 0.0017 % |
| the island from 2.5 km | 0.929 → 0.928 % | 0.0010 → 0.0010 % |
| along the beach | 0.252 → 0.253 % | 0.0017 → 0.0017 % |
| the coast east | 0.480 → 0.480 % | 0.0022 → 0.0022 % |
| over the south beach | 0.025 → 0.025 % | 0.0004 → 0.0004 % |
| the round lake | 0.229 → 0.229 % | 0.0011 → 0.0011 % |

## Checks

- **The batch** against 3c651fd: only the island's images change (`island60` on both paths,
  14 172 px, ꟻLIP mean 0.0045; `water60`, 18 016 px, 0.0053); the city's layered ground and
  everything else at 0 px; the occlusion A/B and mesh against fallback at 0 px.
- **Validation:** `tools/validate.sh` clean on both paths (14 runs).
- **Tests:** a contour names its layers over the height one bit each (`forge_core::material`);
  the workspace's tests, clippy, fmt.
