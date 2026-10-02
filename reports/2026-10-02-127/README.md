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
- From far away the bars read darker and fade into the water. The ground's coarser levels lower
  their low crests, the sea's surface is drawn wherever that ground dips under its level, and the
  wet sand is darker. Cutting the river's water out over them changed the far views by ꟻLIP means
  under 0.00002, so it was left out.
- True distributaries leaving the river to reach the sea apart, each its own ribbon.
- Bars on the large lake fans, if wanted.
- Past the widened mouth's banks, the steep sand face where a beach's berm meets the carve
  (`--view 4354,14,-2830,180,-25`) stands a few metres nearer the water.
