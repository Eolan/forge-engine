# #123: fewer, larger rivers (2026-10-01)

D-041's scale: the uplift lowered along three trunk valleys, a lake's bowl on each, the lower
courses graded to the sea, the small rivers brooks (`docs/demos/island.md`, "Fewer, larger
rivers"). Before (left: `--island-basins 0 --island-grade 0 --no-brooks`, the same pixels as
8525e6b from 2.5 km) and now (right), frame 60, `--island 7`. The islands differ, so each view
is the one its own island's log picks for the same thing.

- `basins.png`: from `genesis --spacing 8`, the basins (each of the eight largest in a hue of
  its own, the rest grey) and the overviews.
- `far.png`: the island from 2.5 km (`--view -6500,2500,-1416,-90,-35`).
- `above.png`: from 9.5 km (`--view 0,9500,0,0,-89`).
- `views.png`, by rows:
  - the largest river's mouth from low (before `-5138,4.4,-258,118.4,-20`, now
    `4384,4.0,-2840,-88.9,-20`);
  - a river across the plain (before `2573,14.0,-3219,52.9,-10`, now
    `-4568,9.2,-924,167.0,-10`);
  - up a steep river from low (before `1956,111.4,1965,88.7,4`, now `733,185.4,-1960,90.8,4`);
  - the largest lake (before `-2176,262.7,-856,0,-15`, now `2104,59.0,-704,0,-15`).

**Numbers** (seed 7):
- The basins at the sea at 8 m: 23.1, 17.8, 11.7 and 7.9 km² (11.4, 10.6, 8.5, 7.5, 6.7, 6.3,
  5.4 before).
- In the demo:
  - 52 rivers (50) and 17 mouths (19);
  - the trunks 52, 47 and 40 m wide at the sea;
  - 4 lakes of a hectare or more (11);
  - 32 259 river points (38 534) and 4 226 stones (5 640).

**Cost**, 2560 × 1440, 1 500 frames:
- From 2.5 km: 3.54 → 3.56 ms.
- Over the plain from 200 m: 2.58 → 2.63 ms, the software raster drawing more of the new
  ground.

**Checks:**
- The batch changes the island's images only (`island60` ꟻLIP mean 0.060, `island8-60` 0.060,
  `water60` 0.10). The A/B harness, streamed against resident and mesh against fallback stay at
  0 px.
- `validate.sh` is clean. 217 tests pass, clippy and fmt are clean.
