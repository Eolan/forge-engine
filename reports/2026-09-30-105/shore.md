# #105 step 5: the shore (2026-09-30)

D-038's shore, in its order: the waves feel the floor, the shore's own waves come in and
break, then the wet sand.

## The damping (644dd4e)

`shore-damping.png`: the coast view (top) and a low view 80 m off the beach (bottom), before
(left) and after (right).

- The surface reads the island's floor and coast distance (`WaterShore`, an `RG16F` image of
  the 8 m field) and damps each cascade by the root of Kitaigorodskii's TMA factor. The factor
  is taken at the frequency the cascade's energy centres on (periods of 7.3, 3.5 and 1.2 s),
  relative to the 50 m the spectrum was made for.
- 150 m off the beach (6 m deep) the swell's cascade keeps 46 % of its height. The sand that
  showed through where its troughs dipped under the floor is gone.
- `water/surface` +0.008 ms at the coast, +0.010 ms from the sea.

## The shore's waves

`shore-waves.png`: the south beach from 60 m up (`--view 0,60,5160,0,-60`) at 10, 12 and 14 s.
`shore-beach.png`: the first of them at full size. `shore-low.png`: the low view before the
shore (left), and at 10 and 13 s.

- **Three Gerstner trains** (9, 7 and 12 s; 0.9, 0.6 and 0.5 m in deep water) come in along
  the coast distance's gradient.
  - Their phase is ω (t + τ), τ the time a crest takes to the shore. `forge_procgen::shore`
    tabulates it from the floor's mean depth against the coast distance and the dispersion
    relation (47 s from 200 m out), so the crests slow and bunch up in the shallows.
  - They shoal (`√(cg₀ / cg)`) until together they stand over 0.78 of the depth, then follow
    the depth down: a surf zone about 40 m wide on the south beach.
  - The sets vary along the coast and the crests drift out of line, from two slow noises.
  - They fade in under a fifth of their deep wavelength of depth, and out where they are
    shorter than four mesh spacings or pixel footprints.
- **Foam:** on the front of each broken crest, left behind as it moves on, and at the swash's
  thin edge; broken up by a drifting noise that fades to its mean below two pixels.
- **The swash:** each wave runs up the beach as a thin sheet, a quick uprush and a slow
  backwash.

**Stability** (still camera, waves held still, TAA on; pixels changing by more than two
levels):

| View | step 4, 1 frame | now, 1 frame | step 4, 32 frames | now, 32 frames |
|---|---|---|---|---|
| coast | 1.62 % | 1.46 % | 0.0021 % | 0.0019 % |
| from the sea | 0.98 % | 0.83 % | 0.0003 % | 0.0001 % |
| the beach from 60 m up | 0.038 % | 0.019 % | 0.0004 % | 0.0002 % |

**Cost** (three runs each, alternating, against step 4):

| View | water/surface | frame |
|---|---|---|
| coast | 0.076 → 0.136 ms | 1.65 → 1.72 ms |
| from the sea | 0.090 → 0.164 ms | 1.45 → 1.52 ms |
| the beach from 60 m up | 0.095 → 0.204 ms | 1.64 → 1.76 ms |

The trains cost about 0.03 ms in the vertex shader and 0.03 ms in the fragment shader from the
sea, where the whole coast's shallows are in view. The levers: an index buffer for the surface
(each vertex is shaded six times today), and trains that start nearer the shore.

**For the owner's eye:**
- From low down the surf is seen edge on and reads as a light band along the beach.
- The white water is white, but the shallows over the sand are nearly as bright.
- The crest that curls over is D-038's later step for the golden shots.

**Checks (the waves, 8f115f2):** the batch changes only the four water images (ꟻLIP mean 0.040; #71's flake aside, on both
paths this time, ꟻLIP mean 0.0013 at most); the occlusion A/B and mesh against fallback are at
0 px; `tools/validate.sh` is clean; tests (two new in `forge_procgen::shore`), clippy, fmt.

## The wet sand

`shore-wet.png`: the beach from 60 m up at 14 s, before (left) and with the wet sand (right).
`shore-wet-zoom.png`: the same, closer.

- **Where:** the swash is a function of position and time shared by the water and the ground
  (`shaders/shore.slang`). The layered ground takes the highest reach over the last 19.5 s (14
  instants), each dimmed by the drying since (25 s to lose 1/e), damp for 0.25 m of height
  above it. The floor under the sea is left to the water.
- **What:** the albedo halves; the surface turns smooth, with water's F0 and a sharp sky
  reflection and sun highlight (Lagarde 2013).
- **Against D-038:** the decision named a clip texture the water writes and the terrain reads.
  The functions give the same decaying maximum run-up without the pass, the clip's edge or its
  resolution, deterministically.
- **For the owner's eye:** the swash's edge and the wet band follow the beach's height
  contours, which step along the 8 m field's cells near sea level. The sand and grass boundary
  above them steps the same way. The 2 m amplification is what would smooth them.

**Cost** (three runs each, alternating, against the waves' commit): `shading/layered`
0.382 → 0.396 ms at the coast, 0.207 → 0.203 ms from the sea (nothing measurable), 0.465 →
0.510 ms above the beach, where most pixels are within the swash's reach. These runs found the
GPU a little slower than the earlier ones; the pairs alternate, so the differences hold.

**Stability** (still camera, waves held still): the beach from above 0.020 % frame to frame
(0.016 % dry: the wet sand's sharper highlight), 0.0001 % over 32 frames either way; the coast
1.32 % and 0.0022 % (1.31 % and 0.0019 % dry).

**Checks:** the batch changes the water images by the wet band (9 046 px, ꟻLIP mean 0.0031),
#71's flake aside; the occlusion A/B and mesh against fallback are at 0 px; `tools/validate.sh`
is clean; tests, clippy, fmt.
