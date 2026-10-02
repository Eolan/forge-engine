# #120: the rivers' deltas into the lakes (2026-10-02)

D-041's lake entry, the last of #120's leftovers on the inflows (`docs/demos/island.md`, "The
rivers' deltas into the lakes"). Before (left, 1606253, the same as `--no-deltas` to the pixel)
and now (right), frame 60 with the fixed step, `--island 7` drawn at 2 m:
- `into-lakes.png`, by rows:
  - the trunk into the north-east lake from 40 m up it (`--view 1504,41.1,-1290,-107.2,-20`);
  - the same from 4 m over its water (`--view 1513,33.1,-1287,-107.2,-8`);
  - the same from 40 m up (`--view 1470,69,-1300,-107.2,-35`);
  - a brook on the plain into its bay (`--view 2303,41.1,-890,-47.0,-20`);
  - a brook from the south into the same lake (`--view 1840,41.1,-712,0,-20`).
- `from-above.png`: the trunk's, the bay's and the south brook's mouths from 70 m straight down
  (`--view 1560,99,-1272,0,-89`, `2340,79,-925,0,-89`, `1840,79,-765,0,-89`).

The demo logs the two longest as `the rivers' deltas into the lakes (--view)`.

**What changed:**
- **The water eases flat to the lake's level** over 8 m and five widths before the lake's edge:
  `L + (z − L)(2t − t²)` at `t` of the reach up from the edge, only ever lowered. The valleys'
  carve follows it.
- **The channel widens** over the same reach as a trumpet, to twice its width at the edge, two
  fifths shallower.
- **A fan on the lake's floor** in front of the mouth, 6 m and three and a half widths long:
  - its top 0.3 m under the water at the mouth and 1.1 m at its far end;
  - its front falling at 0.3 to the lake's floor;
  - painted with a new layer of silty sand inside its outline.

| View | ꟻLIP mean |
|---|---|
| the trunk from 40 m up it | 0.141 |
| the trunk from 4 m | 0.162 |
| the trunk from 40 m up | 0.125 |
| the trunk from above | 0.178 |
| the bay from low | 0.118 |
| the bay from above | 0.118 |
| the south brook from low | 0.062 |
| the south brook from above | 0.082 |

The close views change beyond the delta too. The valleys' carve follows the eased water, and the
rivers traced again over the carved field move by a few metres over their last 50 m: the logged
junction views moved from `1504,-1290` to `1506,-1294` and from `2303,-890` to `2296,-891`.
Their steps, stones and the valley's paint move with them. The batch's far views change in 0.15 to
0.19 % of their pixels.

**Numbers:**
- 6 deltas, fans of 48, 23, 22, 17, 16 and 15 m: the trunk, 11.9 m wide, into the north-east
  lake, five brooks of 2.7 to 4.9 m into it and into the south one.
- 133 texels of sand (4 m).
- 71 271 cells of 8 m refined (71 220); the 2 m ground's 1 140 336 (1 139 520).
- The valleys' carve lowers 44 666 samples (44 625).

**Checks:**
- The capture batch changes the island's images only: `island60` 6 037 px (ꟻLIP mean 0.0015),
  `island8-60` 8 205 px (0.0016), `water60` 5 613 px (0.0019).
- Within the batch the A/B harness, the streamed island against resident and mesh against
  fallback are at 0 px.
- `validate.sh` is clean. 252 tests (one new: a river through a bowl's lake by its delta), clippy
  and fmt pass.
- `timings.sh`: every view within noise.

**Left for later:**
- One channel runs in; D-041's research lets a large fan split into distributaries.
- Above the water the delta is the valley's floor: no bars of sand at the mouth.
- The outlets' two leftovers of the first part. At the north-east lake's outlet the lake's water
  still ends in a soft wavy edge across the river's channel, its 8 m mask's.
