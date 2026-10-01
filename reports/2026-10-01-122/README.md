# #122: the steep rivers' steps and pools (2026-10-01)

Before (left, `--no-steps`: the same pixels as a2d7cca on four views) and now (right), frame 60
with the fixed step, `--island 7` drawn at 2 m:
- `steep.png`: up the steep river from low (`--view 1956,111.4,1965,88.7,4`, logged as `up a
  steep river from low`), and the highest step on a river 5 m wide or more from 15 m down its
  pool (`--view -915,131.0,-3049,-118.5,-5`, logged as `the steep rivers' steps and pools`).
- `head.png`, `head-zoom.png`: a stream near the largest river's head from low (`--view
  -939,325.0,-177,126.5,-6`), and closer.
- `from-above.png`: that stream from above (`--view -925,345,-165,126.5,-60`), the steep river
  from 40 m (`--view 1956,150,1965,0,-89`) and the logged step from 30 m (`--view
  -915,161,-3049,0,-89`).
- `far.png`: the island from 2.5 km (`--view -6500,2500,-1416,-90,-35`) and the plain from 200 m
  (`--view 0,200,5400,0,-8`).

The far water, checked first (before any change):
- `far-light.png`, a crop of the island from 2.5 km:
  - as drawn;
  - without the water's sun highlight: the pale far rivers become dark blue-grey lines, and the
    lake dark blue;
  - without its bed (the same as drawn);
  - without the water: the beds' pale gravel.
- `far-mirror.png`, a crop of the plain from 200 m: as drawn, where the far river mirrors the
  hills behind it, and with the sky alone (`--no-ray-reflections`), a silver thread.

**What it was.** A third of the river points fall more than 4 % (6 459 of 17 652, up to 54 %).
Their water fell evenly, a slide with white streaks painted on it.

**What changed** (`docs/demos/island.md`, "The steep rivers' steps and pools"):
- **Pools and steps:** over 4 % the water stands in pools between steps. They are a width apart
  at 4 % and 0.4 of one from 15 % (the island's widths are three times nature's), 3 m at least
  before a jitter of half again or half as long, and 2 m high at most.
- **Each pool** stands at the level the water had at the next lip, never higher than it was.
- **Each fall** is four points, its line bowed downstream by an arch and a slant.
- **The pool under it** is scoured by H/L/S, 1.8 at 4 % and 1.3 from 15 % (Abrahams et al.
  1995: one to two).
- **The bed:** the banks rise from the level before the steps. Each segment below a lip carves
  nothing upstream of its own start, and the fall's carve bows as its water does.
- **Each lip** carries a row of boulders about the step's height over about 55 % of the width,
  with a gap the fall pours through.
- **The water:** white down the fall and boiling across a third of the pool, in chutes across
  the river.
- **Unchanged:** the 8 m field, the stones off the steps, and every river without steps
  (`--no-steps`).

| View | ꟻLIP mean |
|---|---|
| up the steep river from low | 0.41 (a lip's boulders now stand where the camera looks) |
| the logged step from 15 m | 0.089 |
| the head stream from low | 0.035 |
| the head stream from above | 0.034 |
| the steep river from 40 m | 0.12 |
| the logged step from 30 m | 0.057 |
| the island from 2.5 km | 0.0089 |
| the plain from 200 m | 0.0074 |

**Numbers:**
- 5 805 steps on 27 rivers, 0.54 widths apart on average, 0.70 m high on average and 2.00 m at
  most.
- The deepest water 2.18 m, in a plunge pool (0.75 m).
- 38 534 river points (17 652) and 25 740 stones (1 843).
- 86 919 cells refined (86 922).

**Cost**, at 2560 × 1440 (the same build with and without `--no-steps`, two runs each, 1 500
frames):
- the head stream: 2.82 ms both ways;
- the island from 2.5 km: 3.50 → 3.59 ms (`water/surface` 0.343 → 0.373, the stones' instance
  cull 0.10 → 0.11);
- up the steep river: 4.95 → 4.20 ms, because a lip's boulders hide much of the water there.

**Left for later:**
- **Standing waves** on the 2–4 % rapids as displacement. They need a finer ribbon near the
  camera than the present points and four quads across.
- **The lips are bowed lines,** not each boulder's own shape: the water falls along the line
  between the rocks.
- **The pools' water** is the river's colour over the bed's gravel; deep pools could darken
  more.
