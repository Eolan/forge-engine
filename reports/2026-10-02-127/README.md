# #127: the bars in the large mouths (2026-10-02)

D-041's mouth rule: "distributaries split around bars where the catchment is large"
(`docs/demos/island.md`, "The bars in the large mouths"). Before (left, `--no-bars`) and now
(right), frame 60 with the fixed step, `--island 7` drawn at 2 m:
- `mouths.png`, by rows:
  - the largest mouth from 40 m back up the river, 15 m over its water
    (`--view 4249,15.1,-2810,-75.3,-20`), ꟻLIP mean 0.055;
  - the same from straight over its bars (`--view 4335,119.1,-2832,0,-89`), 0.061;
  - the second largest mouth from 40 m back (`--view -4841,15.5,18,126.5,-20`), 0.067;
  - the same from straight over its bars (`--view -4904,93.2,65,0,-89`), 0.076.
- `sea-far.png`:
  - the largest mouth from 4 m over its water, 30 m back (`--view 4385,4.0,-2839,-88.7,-20`),
    0.028;
  - the same from 500 m up the valley, 80 m up (`--view 3733,80,-2680,-75.8,-8`), 0.003.

The demo logs the two largest mouths' views as `the bars in the large mouths (--view)`.

**What changed:**
- **Where:** a river 20 m wide or more at its mouth at the sea has a bar of sand, 40 m or more
  two.
- **The bars:** teardrops along the river, a blunt head upstream and a long tapering tail, over
  two and a half of its widths, ending a third of a width short of the mouth. Each is 0.3 of the
  width broad, give or take a quarter, shortened, staggered and turned a little on its own.
- **The ground:** their crests stand 0.3 m over the water, rising 1 in 12 out of it and falling
  1 in 3 under it to the channels' beds. The water stays level across, so the sand rising
  through it draws the channels round the bars.
- **The river:** it widens by the bars' breadths over their length, so each channel keeps its
  share of the water.

**Numbers:**
- 4 mouths with bars: two in each of the three largest (the longest 99, 78 and 66 m), one in the
  fourth (72 m).
- The widest river 86 m (52 before).
- 10 more refined cells of 8 m, 160 more of 2 m.

**Checks:**
- The batch changes the island's images only (ꟻLIP means 0.0020–0.0026, about 9 000 px each).
  That includes #120's hill-outlet fix on freshly cooked ground: the batch before drew the old
  carve from the tile cache.
- The A/B pairs are at 0 px; `validate.sh` is clean; 254 tests (one new).
- `timings.sh` is within noise. At 2560 × 1440 a mouth with bars in view takes 0.03–0.07 ms more,
  its water wider (`docs/PROFILE.md`).

**Left for later:**
- ~~From far away the bars read darker and fade into the water.~~ Fixed the same day, below.
- True distributaries leaving the river to reach the sea apart, each its own ribbon.
- Bars on the large lake fans, if wanted.
- Past the widened mouth's banks, the steep sand face (`--view 4354,14,-2830,180,-25`) stands a
  few metres nearer the water. Corrected later the same day: no berm, but the river's own
  bank climbing the metre up to the coastal plain over 2–4 m (`docs/demos/island.md`).

## From far away (2026-10-02, later)

Past a few hundred metres the bars faded into the water. Before (left, 1589834) and now
(right), frame 60, cropped round the mouth:
- `far.png`: the largest mouth from 500 m up its valley, 80 m up
  (`--view 3733,80,-2680,-75.8,-8`): 200 px change, ꟻLIP mean 0.000034 over the frame, up to
  0.15 on the bars;
- `far-second.png`: the second mouth from 460 m up its valley
  (`--view -4471,80,-256,126.5,-8`): 236 px, up to 0.30.

**What it was.** Each suspect was ruled out by switching it off:
- **The ground's coarser levels** (`--lod-error 0.05`): almost no change.
- **The sea's surface:** shown in false colour, it stands under the bars' crests.
- **The wet sand:** switched off, almost no change.

The river's own water was the cause. Far away its ribbon lies on the ground over its whole
width, lifted a pixel and a half and drawn whatever the depth under it, and it covered the bars.
At the largest mouth's bars, a pixel is 116,120,120 with the river's water, 128,121,112 without
it, and unchanged without the sea's.

**What changed.** `forge_procgen::bar_spans` gives every ribbon point the span of each of its
first two bars across it, at the water's edge. The river's points carry it to the GPU (112
bytes a point, from 96), and the river's water is drawn nowhere inside it.
The test of the bars now checks the spans against the outlines.

**A correction.** The same cut-out was tried while the bars were built and dropped as useless.
That judgement read the whole frame's ꟻLIP mean, which a few hundred pixels cannot move.

**Checks:**
- The capture batch changes `water60` only: 188 px, ꟻLIP mean 0.00003, max 0.059, the same on
  the mesh path and the fallback.
- The A/B pairs are at 0 px, and `validate.sh` is clean.
- 254 tests pass; clippy and fmt pass.
- `timings.sh` is within noise (the island 1.582–1.585 ms against 1.579–1.606).
- Near, the views change only on the bars' sand:
  - 15 m up, 15 px;
  - straight above, 57 px;
  - 4 m over the water, 1 355 px, where a faint sheet of water had lain over the far end of a
    bar's head.
