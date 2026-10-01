# #108: under the sea (2026-10-01)

The view from under the sea's surface, the first part of the underwater view
(`docs/demos/island.md`, "Under the sea" and "Caustics"):
- the water at the camera;
- the surface from below, with Snell's window;
- the water's light along the view ray;
- the waterline across the lens;
- the waves' caustics on the floor.

Seed 7 (`--island 7`), frame 60 with the fixed step, 1600 × 900.

`views.png`, by rows (before the caustics):
- across the floor towards the shore, 3 m under (`4770,-3.0,-2847,91.1,-10`, the log's
  `under the sea` view), and up at Snell's window from there (`4770,-3.0,-2847,91.1,40`);
- towards the sun (`4770,-3.0,-2847,-60,35`), and the floor from 6 m under
  (`4770,-6.0,-2847,91.1,-30`);
- the waterline across the lens from 0.48 m (`4770,0.48,-2847,91.1,0`), and from 0.45 m, where
  the lower part of the frame looks at the wave in front from inside it.

`caustics.png`: the floor from 3 m and from 6 m under the sea, without (`--no-caustics`, left)
and with the caustics (right).

`shallows.png`: the shallows off the first view's beach from 25 m (`0,25,5214,0,-20`), without
and with the caustics, at full resolution.

**Cost** (2560 × 1440, 1 500 frames, two rounds):
- The view from below:
  - from 2.5 km and over the plain from 200 m, unchanged (3.51 and 2.61–2.63 ms);
  - from 4 m over the largest mouth, 3.24 → 3.26 ms;
  - 3 m under the sea, 3.15 ms: `water/under` 0.095 ms, `water/at-camera` 0.018 ms.
- The caustics, in `shading/layered`:
  - from the coast, +0.06 ms;
  - from 2.5 km, nothing;
  - across the floor 3 m under, +0.09 ms;
  - with the floor filling the view, +0.12 ms.

**Checks:**
- The view from below: the capture batch at 0 px (42 images), and four views within 20 m of the
  sea at 0 px against the previous commit.
- The caustics: the batch changes the four water images only (the coast's shallows). With
  `--no-caustics` they match the previous commit to the pixel.
- The A/B harness, streamed against resident and mesh against fallback: 0 px.
- `validate.sh` is clean, with its new run under the sea on both paths. The tests pass (217);
  clippy and fmt are clean.
