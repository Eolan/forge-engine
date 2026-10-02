# #107: objects in the water (2026-10-02)

The first part: the rivers part around what floats in them, the flow taken relative to each thing
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

## The second part: wakes in still water (wave particles)

What moves through the lakes and the sea leaves waves (`docs/demos/island.md`, "The wakes"):
wave particles carrying packets of waves 0.5 m long, on the async compute queue, splatted into a
field around the camera whose slopes the water adds. Frame 300 with the fixed step, 1600 × 900,
1 000 barrels, the last one towed round a 20 m circle on the largest lake at 2.5 m/s.

- `towed.png`: the towed barrel from the log's `towed` view
  (`2168.0,36.90,-1234.0,-102.9,-36.2`): without the wakes (`--no-wakes`), with them, the pixels
  that changed. Crests ring its bow and trail 15 m behind it along its curve; 35 138 pixels,
  ꟻLIP mean 0.0070, max 0.99 where a crest catches the sun.
- `towed-low.png`: the same from 2 m over the water and 16 m away
  (`2163.1,30.9,-1235.1,-102.9,-7`): fine lines of ripples in its wake; 9 987 pixels, ꟻLIP mean
  0.0012.

**Cost** (2560 × 1440, two rounds): the frame 0.08 ms more (3.60–3.62 → 3.68–3.70 ms from the
towed barrel); on the compute queue `wakes/advance` 0.08–0.11 ms, `wakes/emit` 0.014–0.018,
`wakes/slopes` 0.010, `wakes/clear` 0.004.

**Checks:** the same capture twice is the same to the pixel; without movers the batch is at
0 px; `validate.sh` is clean with the wakes; tests, clippy and fmt pass.
