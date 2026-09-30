# #105 step 6: the rivers (2026-09-30)

D-038's rivers: with `--water`, the island's rivers are water of their own, ribbons made from
stage 4's courses (`forge_procgen::river`) and drawn in `water/surface` after the sea.

- `rivers-stream.png`: a stream on the eastern plain from 16 m up (`--view
  2863,250,-1174,-21.7,-15`): before (the rivers painted into the ground's layers), and now.
- `rivers-plain.png`: the same streams meeting, from 40 m up (`--view 2971,274,-1174,68.3,-20`).
- `rivers-valley.png`: the largest river down its valley to the west coast, from 200 m up
  (`--view=-5400,200,-1416,-90,-30`).
- `rivers-island.png`: the island from 2.5 km up in the west (`--view=-6500,2500,-1416,-90,-35`).

## How it works

- **The courses:** the drawn field's 43 rivers, each from its head to the sea or to the river
  it joins.
  - Four passes of a binomial filter and three of Chaikin's corner cutting, then a point every
    4 m (16 397). The D8 grid's stair steps straighten, and its right-angled turns open into
    bends.
  - Per point: the width from the catchment (3.5 to 14 m), the depth `0.3 (A / km²)^⅜` m
    (0.23 to 0.65 m), the speed by Chézy's formula over the bed's slope (0.3 to 3 m/s).
  - In a bend the half width stays under 0.8 of its radius. A river fades in over its first
    40 m.
- **On the ground, not in a bed:** the 8 m field cannot hold a channel a few metres wide, and
  water in a carved trench would outline the triangles. So the ribbon lies on the ground as the
  mesh draws it, each vertex at the highest the ground reaches under the quads around it (sampled
  every half metre), a little above. The depth the water shows is the ribbon's own profile.
  With the 2 m field (#106) the beds can be carved.
- **In the pass:** blended over what is under by how much of the pixel the river covers: banks
  soft over half a metre or two pixels, the last 30 m to the coast given to the sea, and far
  away at least a pixel either side with the coverage scaled to the river's share (after
  Persson's phone-wire anti-aliasing). Tributaries first; the larger river covers their ends.
- **The water:** the finest cascade's slopes carried downstream by a flow map (Vlachos 2010),
  the sea's sky, sun, mirror and shadow rays, a grey-brown sediment bed towards the middle seen
  through the water's depth, white water in riffles where the bed falls steeply.
- **The ground:** with `--water` the rivers are no longer painted into the layer map; the
  lakes still are, until their step.

## For the owner's eye

- **From above,** a river over the plain shows its bed and a faint sky: greyer than the painted
  stand-in (`rivers-plain.png`), which mirrored the sky more than water does. The levers, if it
  should read bluer: a smoother surface at a distance, or a deeper look to the water.
- **The steep valleys:** the ribbon rests on the highest ground under it, so in a narrow 8 m V
  its water can stand up to about a metre over the valley's bottom. From low down across such a
  valley, it shows as a sheet a little above the ground.
- **The valleys' sides** step every 8 m (#106), and their shadows cross the rivers in stripes.
- **The lakes** are still the painted stand-ins; the rivers cross them.
- **No waterfalls:** the eroded field has no cliffs (the steepest reach falls 33 % over 24 m).
  The steep reaches have white water instead.

## Numbers

**The frame** (3 000 frames, three runs each, alternating, the waves held at 12 s), the wet
sand's commit against this one:

| View | frame | water/surface | water/reflections |
|---|---|---|---|
| coast (a river mouth far off) | 1.721 → 1.736 ms | 0.136 → 0.153 | 0.208 → 0.208 |
| the valley from 200 m | 1.809 → 1.840 ms | 0.142 → 0.162 | 0.120 → 0.121 |
| the plain from 40 m | 1.536 → 1.577 ms | 0.058 → 0.079 | 0.017 → 0.035 |
| the stream from 16 m | 1.457 → 1.541 ms | 0.056 → 0.082 | 0.017 → 0.050 |
| the island from 2.5 km | 2.176 → 2.206 ms | 0.179 → 0.195 | 0.080 → 0.081 |

- About 0.017 ms goes to the ribbons' 390 000 vertices, drawn whether in view or not; a draw
  per stretch of river, culled by its bounds, is the lever.
- At start, 77 ms; on the GPU, 17 MiB (the ground's heights in full precision, and the points).

**Stability** (pixels changing by more than two levels, still camera, waves held, TAA on):

| View | before, 1 frame | now, 1 frame | before, 32 frames | now, 32 frames |
|---|---|---|---|---|
| the stream from 16 m | 0.18 % | 0.26 % | 0.0017 % | 0.0020 % |
| the plain from 40 m | 0.31 % | 0.33 % | 0.0019 % | 0.0021 % |
| the valley from 200 m | 0.59 % | 0.59 % | 0.0150 % | 0.0146 % |
| the island from 2.5 km | 0.87 % | 0.82 % | 0.0003 % | 0.0006 % |
| coast | 1.39 % | 1.39 % | 0.0023 % | 0.0026 % |

The ripples' reflections move with the jitter from one frame to the next; nothing crawls.

## Checks

- **The batch** against the wet sand's: the four water images change by a river mouth and a
  gully in the first view (506 px, ꟻLIP mean 0.0003); #71's flake (`fb-ast-taa600`, 222 px,
  ꟻLIP mean 0.0009); everything else at 0 px. The occlusion A/B and mesh against fallback are at
  0 px. The sea's shading, now shared with the rivers, is unchanged to the pixel.
- **Validation:** `tools/validate.sh` is clean on both paths.
- **Tests** (two new in `forge_procgen::river`: a valley's ribbon, and a tributary ending on
  its river with its bends open; the ribbon never under the drawn ground), clippy, fmt.
