# #105 step 7: the lakes (2026-10-01)

D-038's last step: the island's 15 lakes of a hectare or more, painted into the layer map until
now, are water of their own with `--water`. Each sheet is before (left, the rivers' commit
4f8f54c, the painted stand-in) and now (right), `city-blocks --island 7 --water`:
- `lakes-west.png`: the largest lake from 30 m over its south shore (`--view=-2140,308.1,-808,0,-15`),
  a river running in on the left.
- `lakes-east.png`: a long lake on the eastern plain (`--view 2928,297.4,-1720,0,-15`).
- `lakes-round.png`: a round lake further north (`--view 2972,284.5,88,0,-15`): its shore a
  smooth curve on cells of a metre where the 8 m triangles drew a polygon.

## How

`docs/demos/island.md`, "The lakes":
- a level plane per lake over its mask (the priority flood's depression at its level, its
  shallow margins too, and a sample more), the ground rising through it drawing the shore;
- the shore's cells refined to a metre on the smoothed ground, as the rivers' channels are;
- the water the rivers' (one shading function now draws both), still but where a river runs in:
  the 20 river inflows join the mouths' list and their jets carry the ripples into the lake;
- the lakes drawn before the rivers, so a river's ribbon fades over its last points into the lake
  at the lake's own level: two surfaces with the same ripples, level and water;
- silt under a lake wherever its plane stands over the ground; no rocks in the lakes.

## For the owner's eye

- **The shallows:** the depression at the lake's level reaches up the inflowing valleys, where
  the water is centimetres deep over silt.
- **The far shores** still show the stepped 8 m slopes beyond the refined band (#106).
- **The mask's edge** is softened over half a sample; where the ground stands under the level at
  the edge of a mask (an outlet's channel), the plane ends there.

## Numbers

**The frame** at 1600 × 900 (3 000 frames, three runs each alternating, the waves held at 12 s),
the rivers' commit against this one:

| View | frame | water/surface | water/reflections | shading/layered |
|---|---|---|---|---|
| the west lake from 30 m | 1.559 → 1.611 ms | 0.071 → 0.088 | 0.033 → 0.067 | 0.362 → 0.377 |
| the east lake | 1.411 → 1.454 ms | 0.070 → 0.080 | 0.024 → 0.047 | 0.302 → 0.324 |
| the round lake | 1.490 → 1.577 ms | 0.071 → 0.094 | 0.024 → 0.075 | 0.320 → 0.375 |
| coast | 1.684 → 1.676 ms | 0.155 → 0.155 | 0.198 → 0.186 | 0.341 → 0.337 |
| the island from 2.5 km | 2.125 → 2.135 ms | 0.194 → 0.195 | 0.081 → 0.082 | 0.295 → 0.293 |
| down a river from 3 m | 1.569 → 1.559 ms | 0.093 → 0.093 | 0.120 → 0.113 | 0.377 → 0.371 |

- Most of a lake's cost is its rays: a mirror and a shadow ray for every pixel of water, as the
  sea's and the rivers' (`water/reflections` +0.02 to 0.05 ms).
- The silt under the shallows blends with the grass's layer (`shading/layered` up to +0.055 ms).
- The plane itself: 15 quads.

**Stability** (pixels changing by more than two levels, still camera, waves held, TAA on):

| View | before, 1 frame | now, 1 frame | before, 32 frames | now, 32 frames |
|---|---|---|---|---|
| the west lake | 0.36 % | 0.50 % | 0.0022 % | 0.0012 % |
| the east lake | 0.24 % | 0.27 % | 0.0019 % | 0.0015 % |
| the round lake | 0.33 % | 0.24 % | 0.0025 % | 0.0015 % |
| coast | 1.41 % | 1.41 % | 0.0026 % | 0.0025 % |
| the island from 2.5 km | 0.80 % | 0.80 % | 0.0010 % | 0.0010 % |
| down a river from 3 m | 0.42 % | 0.42 % | 0.0006 % | 0.0035 % |

The lakes' ripples move their reflections with the jitter from one frame to the next (the west
lake 0.36 → 0.50 %); over 32 frames nothing crawls.

**At start:** the lakes' masks with the rivers' water (0.75 s as before, twice); the cook's
channels and shores 0.8 s.

## Checks

- **The batch** against the rivers' commit: only the island's images change (26 633 px, ꟻLIP
  mean 0.0061: the shores smoothed and the rocks out of the lakes; with the water 31 296 px,
  0.0070); everything else at 0 px; the occlusion A/B and mesh against fallback at 0 px.
- **Validation:** `tools/validate.sh` clean on both paths.
- **Tests:** a basin's water covers its depression and a sample more and nothing beyond its rim
  (`forge_procgen::lake`), and the channels' tests with the shores' smoothing in place; clippy,
  fmt.

## Later the same night: darker lakes

From the air (the island from 2.5 km) the lakes read as pale patches: shallow water shows its bed,
and the bed was the sea floor's sand-coloured silt under the rivers' clear water. The lakes now
have a bed of dark mud (`island: lake bed`) and water of their own, darker than the rivers'
(absorption 1.0, 0.6, 1.2 m⁻¹), mixing to the rivers' along an inflow's jet.
`lakes-dark.png`: the island from 2.5 km and the round lake, before (left) and now (right). The
batch is unchanged (0 px: no lake in its views); validation clean.

## Later the same night: no river channel through a lake

From over the lakes' outlets and inlets, the rivers' carved channels ran on through the lakes'
beds as dark trenches under the water (`lake-outlets.png`, top: the three largest lakes, from
25 m over their outlets; the logged `the largest lakes' outlets` views). A river's channel now
stops where its course is under a lake's water at both ends of a segment
(`ChannelParams::carve_lakes`, false): the lake's bed is its own (bottom).
