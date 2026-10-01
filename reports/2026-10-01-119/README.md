# #119: the confluences' corners rounded (2026-10-01)

Before (left, a333254) and now (right), frame 60 with the fixed step, `--island 7` drawn at 2 m:
- `confluence.png`: the logged confluence (`--view 3038,19.1,1409,-87.2,-20`), and from its
  other side (`--view 3090,19.1,1405,92.8,-20`); `confluence-zoom.png`, its junction closer.
- `from-above.png`: the logged confluence from 60 m (`--view 3062,70,1409,0,-89`), a junction
  at right angles on the plain (`--view 1698,49,-3524,0,-89`) and one near the coast (`--view
  -4936,43,-309,0,-89`), from 40 m.
- `junctions.png`: the right angle from low (`--view 1698,24,-3484,0,-20`), the coast's
  junction (`--view -4936,15,-269,0,-20`) and a hill junction (`--view -2677,124,-430,0,-20`).

The demo logs the four largest tributaries' junctions from 40 m as `the confluences' corners
rounded (--view)`.

**What it was.** Each river's channel was carved on its own and the lowest kept, so where a
tributary met its river their banks met in a corner either side, the ground's and the water's
edge alike.

**What changed** (`docs/demos/island.md`, "The confluences' corners"):
- **A circle rounds each corner:** it touches both rivers' water's edges, their own curves near
  the junction, 2 m and the tributary's width from where they met (`RibbonParams::confluence`).
  At the logged confluence that is 8.6 m for the acute corner and 45 m for the obtuse one.
- **The ground:** the bank rises from the arc as the banks it touches do. Between the arc and the
  old corner a shallow bed falls as the rivers' beds do at their edges. Past the old edges it
  blends into their beds, so the old corner is gone under the water without a step.
- **The water:** each part of a corner is drawn whole by the river whose edge is nearer, and by
  the river joined where the tributary's water fades (`RibbonPoint::cover`). The ground draws
  the edge.
- **The water's edge across a ribbon** is interpolated in metres per vertex. It was a share times
  the half width per triangle, which drew teeth where the width changes fast.

| View | ꟻLIP mean |
|---|---|
| the logged confluence | 0.024 |
| from its other side | 0.013 |
| from above | 0.018 |
| a right angle on the plain, from above | 0.018 |
| a junction near the coast, from above | 0.018 |
| the right angle from low | 0.015 |
| the coast's junction from low | 0.014 |
| a hill junction from low | 0.022 |
| the logged confluence from 300 m | 0.008 (the rivers' ripples only) |

**Numbers:** 51 corners at 26 junctions (the others meet their river in a lake or at the sea),
radii 17 m on average and 109 m at most; 86 922 cells of 8 m refined (86 875), 1 390 752 at 2 m
(1 390 000).

**Checks:** the capture batch changes the island's images only (`island60` ꟻLIP mean 0.0023,
`water60` 0.0030, `island8-60` 0.0026); the A/B harness, the streamed island against resident
and mesh against fallback stay at 0 px; `validate.sh` is clean; 214 tests (a Y's corners
rounded: the old corner under the water, the arc its edge, the water covering it, no step on a
2 cm grid around it), clippy, fmt.

**Left for later:**
- The corner's water is a shallow shelf, not the bar and the scour hole of a real confluence.
- At a steep confluence the corner's water runs from the tributary's level to the river's,
  following the two ribbons' levels rather than a surface of its own.
- Near the coast the tributary's clear water fans out beside the river's pale, sea-mixed water.
