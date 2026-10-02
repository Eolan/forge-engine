# #128: the beaches by type (2026-10-02)

Before (left, `--no-beach-types`) and now (right), frame 60, `island` (seed 7):
- **`beaches.png`**, by rows:
  - the longest shingle stretch from 70 m out at sea, 18 m up (`--view 4264,18,1725,32.9,-14.4`),
    ꟻLIP mean 0.055;
  - the longest black sand stretch the same way (`--view -5304,18,-343,-56.3,-14.4`), 0.123;
  - on the shingle, 3 m up, along the beach (`--view 4226,3,1666,122.9,-8`), 0.254;
  - on the black sand (`--view -5266,3,-372,33.7,-8`), 0.242.
- **`above.png`**:
  - the black stretch from 400 m up (`--view -5246,400,-382,0,-89`), 0.031: its ends mix into
    the pale sand in patches;
  - the island from the sea (`--shot island`), 0.0056.

**The rule** (`forge_procgen::paint_beaches`), on the beach band the slope rule paints:
- **Black sand:** where the rock behind is hardest (stage 2's hardness field), a sixth of the
  beaches away from the mouths.
- **Shingle:** on the headlands and under steep land, a quarter of the rest. Its pebbles are a new
  texture (`textures::shingle`); the river beds' cobbles read as paving.
- **Pale sand:** in the bays and within 400 m of a river's mouth.
- **Along the coast:** the fields are read on a 32 m grid and blurred over a few hundred metres,
  so the stretches are long, and their ends mix over tens of metres.
- **Under the sea:** the dark types run 12 m out under the sea.

**The contour** (#106) takes the three beaches under its height, so each one's top follows the
drawn ground. The layered resolve keeps 96 registers and no spill.

**Numbers:**
- **Coast:** 19.7 km of pale sand, 7.4 km of shingle and 3.3 km of black sand.
- **Start-up:** 82 ms for the rule, 60 ms for its logged views. A first version took 2 s, from
  every stage-2 field and a scan of the whole map per step under the sea.

**Checks:**
- **The batch:** only the island's images change. `island60` changes by 19 px, `water60` by 23,
  the `island` shot by 2 539; the other three shots are unchanged.
- **A/B and paths:** 0 px.
- **Validation and tests:** `validate.sh` is clean; 256 tests pass, two new.
- **Timings:** `timings.sh` is within noise.
