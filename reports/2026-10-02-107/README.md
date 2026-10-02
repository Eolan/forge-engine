# #107: objects in the water, the rivers' part (2026-10-02)

The rivers part around what floats in them, the flow taken relative to each thing
(`city-blocks --island 7 --movers N`, `docs/demos/island.md`, "Objects in the water"). Frame 60
with the fixed step, 1600 × 900, 1 000 barrels: carried at the water's speed, one in ten moored.

- `moored.png`: the first moored barrel the water passes at 1.2 m/s or more, from above
  (`-1954.0,51.69,249.9,77.1,-49.6`, the log's `moored` view), cropped and enlarged twice:
  - without the floaters (`--no-floaters`);
  - with them;
  - the pixels that changed: 8 588; ꟻLIP mean 0.0017, max 0.40.
  - The wake runs downstream (to the right) over 3–4 m, white in streaks and a rougher surface.
  - The pillow in front is in place (a debug colour showed it), but the white water's streak
    pattern leaves the 0.3 m in front of the barrel mostly clear, as for the stones.

**Cost** (2560 × 1440, two rounds): `water/surface` 0.069 → 0.081 ms from the moored barrel, no
change beyond the rounds' spread from the largest mouth. Without the grid of 16 m cells around
the camera, every river pixel looping over all 64 floaters, it was 0.10 and 0.34 ms more.

**Checks:**
- Without movers the capture batch is at 0 px, and so are the A/B harness and mesh against
  fallback.
- `validate.sh` is clean, with 1 000 movers on both paths.
- The tests pass; clippy and fmt are clean.
