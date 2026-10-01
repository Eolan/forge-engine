# #108: under the water (2026-10-01)

The view from under the water: the sea's, a lake's and a river's
(`docs/demos/island.md`, "Under the sea", "Caustics" and "Under the lakes and the rivers"):
- the water the camera is in;
- the surfaces from below, with Snell's window;
- the water's light along the view ray;
- the waterline across the lens;
- the waves' caustics on the floor.

Seed 7 (`--island 7`), frame 60 with the fixed step, 1600 × 900.

`views.png`, by rows:
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

`fresh.png`, by rows:
- 1.5 m under the largest lake, looking across (`2248,27.5,-1184,0,0`) and up towards the sun
  (`2248,27.5,-1184,180,30`): its dark water, a metre or two of sight;
- at the lake's level (`2248,29.02,-1184,180,0`), the waterline near the top of the frame, and
  in the largest river by its mouth (`4384,-0.2,-2840,-88.9,5`), its silty water over its bed.

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
- Under the lakes and in the rivers: 9 m under the largest lake 1.99 ms, in the largest river
  2.08 ms; `water/under` 0.08 ms, `water/at-camera` 0.007 ms.

**Checks:**
- The view from below: the capture batch at 0 px (42 images), and four views within 20 m of the
  sea at 0 px against the previous commit.
- The caustics: the batch changes the four water images only (the coast's shallows). With
  `--no-caustics` they match the previous commit to the pixel.
- The A/B harness, streamed against resident and mesh against fallback: 0 px.
- The lakes and the rivers from below: the batch and the four views within 20 m of the sea at
  0 px against the previous commit.
- `validate.sh` is clean, with its new runs under the sea and under the largest lake on both
  paths. The tests pass (217); clippy and fmt are clean.
