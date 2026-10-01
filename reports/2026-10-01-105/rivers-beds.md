# #105: the rivers' beds, level water, mouths and stones (2026-10-01)

After the owner's look at step 6 (`reports/2026-09-30-105/rivers.md`): the water was not level
across (it followed the ground's V), some ribbons stood over the ground or left gaps at the banks,
there was no bed to see (it read as shallow, swampy water), the rivers stopped short of the sea,
and nothing in the water changed its flow. The water now lies level in channels carved into the
island's mesh, over a gravel bed seen through its depth; the rivers run through the beaches into
the sea and mix into it; and stones in them part the flow.

Each sheet is before (left, the step-6 commit) and now (right), `city-blocks --island 7 --water`:
- `beds-down.png`: down a river from 3 m over its water (`--view=-3058,237.4,-742,5.6,-10`):
  before, a sheet of water over the valley's floor; now the river in its channel.
- `beds-stones.png`: a steep stream running to the sea (`--view 2990,91.2,2224,-81.7,-10`):
  stones breaking the water, white water in front of them and wakes behind. Before, this point
  was inside the uncarved ground.
- `beds-stream.png`: the stream on the eastern plain from 16 m up (`--view 2863,250,-1174,-21.7,-15`).
- `beds-mouth.png`: the largest river's mouth from 110 m (`--view=-5080,110,-1340,0,-55`): before,
  the ribbon stopped 30 m short of the sea; now the channel runs through the beach, the sea fills
  its end and the river's water goes out in a plume.
- `beds-valley.png`: the largest river's valley from 200 m (`--view=-5400,200,-1416,-90,-30`).

## How

`docs/demos/island.md`, "The rivers' beds, level water, mouths and stones": cells of a metre
along the rivers (36 267 cells of 8 m, 2.9 M fine vertices) stitched to the 8 m cells without a
crack; the course settled on the valley's floor; a level that only falls and stays under the
banks; a parabolic channel carved under it; a gravel bed layer; the water seen through its true
depth; far away the ribbon still lies on the ground; at the mouths the sea's shading takes the
river on with the same ripple function; 2 525 stones, the water parting around them as a
potential flow around a cylinder.

## For the owner's eye

- **The plume** at a mouth is a thin layer of the river's silty water over the sea's, widening
  and wandering out 60 half widths. It reads as a brownish band from above; it may want to be
  fainter.
- **The white water** behind the stones reads as smooth streaks from a few metres; its pattern is
  0.8 m, the riffles'.
- **From far away** a river now sits in its channel and shows less than the old ribbon did, which
  lay on the ground and mirrored the sky at grazing angles (`beds-valley.png`).
- **Rough ground:** where the 8 m field jumps 5 to 15 m between neighbouring samples (the
  roughness on #106), the level water cuts a gorge through the spurs: a fifth of the points stand
  more than 2 m under their lowest bank.
- **The banks' steps:** the 8 m cells beside the channels keep their facets and their shadows
  (#106); the channel and a metre around it are smooth.
- **A line of foam** along fast water's edge was tried and left out: it drew the carved banks'
  metre triangles.

## Numbers

**The frame** at 1600 × 900 (3 000 frames, three runs each alternating, the waves held at 12 s),
step 6's commit against this one:

| View | frame | water/surface | water/reflections | shading/layered |
|---|---|---|---|---|
| coast | 1.662 → 1.695 ms | 0.148 → 0.154 | 0.196 → 0.200 | 0.342 → 0.343 |
| the valley from 200 m | 1.738 → 1.786 ms | 0.153 → 0.164 | 0.115 → 0.119 | 0.444 → 0.447 |
| the plain from 40 m | 1.495 → 1.530 ms | 0.073 → 0.076 | 0.033 → 0.034 | 0.380 → 0.397 |
| the stream from 16 m | 1.439 → 1.471 ms | 0.078 → 0.082 | 0.044 → 0.043 | 0.349 → 0.369 |
| the island from 2.5 km | 2.082 → 2.122 ms | 0.185 → 0.191 | 0.078 → 0.081 | 0.291 → 0.295 |
| down a river from 3 m | 1.476 → 1.565 ms | 0.089 → 0.092 | 0.137 → 0.120 | 0.288 → 0.377 |
| the mouth from 110 m | 1.720 → 1.777 ms | 0.157 → 0.166 | 0.113 → 0.123 | 0.538 → 0.556 |

- The bed costs the most where it fills the view: its layer blends with the grass's, two
  textured materials a pixel (`shading/layered` +0.09 ms down a river).
- The plume costs 0.006 ms over the coast's sea: a grid of 128 m cells lists the mouths that
  reach each cell (a loop over all 25 cost 0.04 ms).
- The rest of the frame's growth is the geometry: the refined cells and the stones.

**Stability** (pixels changing by more than two levels, still camera, waves held, TAA on):

| View | before, 1 frame | now, 1 frame | before, 32 frames | now, 32 frames |
|---|---|---|---|---|
| the valley from 200 m | 0.59 % | 0.59 % | 0.0146 % | 0.0178 % |
| the plain from 40 m | 0.33 % | 0.31 % | 0.0021 % | 0.0013 % |
| the stream from 16 m | 0.26 % | 0.24 % | 0.0020 % | 0.0009 % |
| the island from 2.5 km | 0.82 % | 0.80 % | 0.0006 % | 0.0010 % |
| coast | 1.39 % | 1.41 % | 0.0026 % | 0.0026 % |
| down a river from 3 m | 1.00 % | 0.42 % | 0.0014 % | 0.0006 % |
| the mouth from 110 m | 0.11 % | 0.09 % | 0.0005 % | 0.0007 % |

**At start:** the rivers' water, channels and far resting heights take 0.75 s (the drainage, the
lakes' flood, the courses, the levels, the channels' index, the heights under the ribbons in
parallel), done for the ground's layers and again for the water; the cook takes the channels'
fine heights once (1.5 s) when the island's mesh is not in the cache.

## Checks

- **The batch** against step 6's: only the island's images change (52 582 px, ꟻLIP mean 0.0099;
  with the water 56 139 px, 0.0106): the first view by the rocks that left the rivers' cells and
  by the stones, the water's also by the rivers' mouths far off. Everything else at 0 px; the
  occlusion A/B and mesh against fallback at 0 px.
- **Validation:** `tools/validate.sh` clean on both paths.
- **Tests:** the refined heightfield has no crack and draws its coarse cells unchanged
  (`forge_geom`); the valley's water falls and stands under its banks; a tributary ends on its
  river at its level; the channel holds the level at the water's edge, the depth in the middle,
  leaves the ground far away, and its fine heights meet the coarse cells; the stones stand on the
  bed within the water, the same every time (`forge_procgen`); clippy, fmt.

## Later the same night: no water over ground not drawn yet

The owner saw ribbons floating "maybe because terrain is not ready". With the island's pages
streamed (the default), the first frames draw the water before the ground under it: frame 3 of
the view down a river showed the river hanging over the sea far below (`streaming.png`, left).
Rivers and lakes now fade where the view ray finds nothing within the deepest they can be (their
own depth, a lake's deepest point, and 5 m more) under their surface (right); once the ground is
in, nothing changes (the batch, captured with the pages resident, at 0 px everywhere; validation
clean).

## Later the same night: finer white water

Close up, the white water in fast reaches was soft blobs a metre across (one octave of value
noise at 0.8 m). It is now two octaves (0.45 and 0.18 m) with a sharper threshold, carried on
the flow, whose shear draws it into lines along the fast water. `white-water.png`: the stone in
the fastest water, by the largest river's mouth (`--view=-5091,4.6,-1395,40.6,-14`, the log's
`stone_view`), before (left) and now (right). The batch at 0 px; validation clean; frame to frame
1.016 → 1.020 % in that view.

## Later the same night: rivers grow from their springs

At its head a river's channel was carved to its full width while its water faded in over 40 m,
so the head read as a dry, pale path up the hill (`spring.png`, left: the largest river's head,
the log's `head` view). A river now grows from its spring over those 40 m, from a sixth of its
width and a third of its depth (`RibbonParams::spring`), and its water fades in over the first
8 m (right).
