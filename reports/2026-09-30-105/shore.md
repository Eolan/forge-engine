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

**Checks:** the batch changes only the four water images (ꟻLIP mean 0.040; #71's flake aside, on both
paths this time, ꟻLIP mean 0.0013 at most); the occlusion A/B and mesh against fallback are at
0 px; `tools/validate.sh` is clean; tests (two new in `forge_procgen::shore`), clippy, fmt.
