# #199: where the areas meet

`island --sw-raster on --fixed-step`, frame 60. Each image is before (above) and after (below).
- `edges.png`: the bank at the river's mouth from 22 m (`--view=4255,22,-2760,-75.5,-22`). The
  sand's edge was cut in straight lines and corners and smeared; now it is broken up.
- `walker.png`: the beach at the walker's height, 7 m up (`--view=0,7,5070,0,-20`). The cuts ran
  straight out from the camera, along the map's grid.
- `salt.png`: the mouth (`--shot mouth`), both with the new edge. Before, the grass ran down the
  banks to the water; now the salt water's sand holds to the bank's top.

**What it is**
- **The contour's share.** It is continuous where the four texels change. The height decides it
  between bounds the texels set, and each side's share goes to its own texels.
- **The edge.** It is broken up by noise 1.7 m and 0.6 m wide, and blends with the pixel's
  footprint. Off the contour, the texels decide.
- **The lookup's wander** gains a second octave a texel wide.
- **`forge_procgen::paint_salt`.** It turns the grass to sand within 40 m of the salt water: up
  to 5 m over the sea at the water, falling to 2.5 m at 40 m. On seed 7 that is 1 392 texels in
  283 ms. `--no-salt` turns it off.

**Costs**
- `shading/layered` is 0.318 → 0.319 ms on the island's view and 0.340 → 0.343 ms on its tour,
  against `ecd826f`.

**Not yet**
- **The river's banks at the mouth:** a straight steep face along the river, and angular notches,
  seen through the shallow water. They come from the carve (`Channels::carved`): the bank rises
  at 0.3 of its slope near the sea and ends in the carve's fade at a constant distance, and a
  hard `min` makes creases.
