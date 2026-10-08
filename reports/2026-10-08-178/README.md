# #178: a straight line across the highest lake (2026-10-08)

The owner saw a straight line across the water in a capture for #177:
`city-blocks --island 7 --fixed-step --view=-238.2,318.14,-1843.9,135.2,-18.1`, frame 60, 1.3 m
over the highest lake (`docs/demos/island.md`, "A line across the lake").

`line.png`: before (left) and after (right); the top row as drawn, the bottom row with
`--clouds 0`, where the two sides' colours stand apart most.

**The cause:** a brook runs into the lake 4 m from the camera (half width 0.2 m). Its channel
turns away at its mouth, so none of it runs straight (`WaterMouth::back` = 0). The plume's water,
the river's colour, flow and ripples, was cut off on the mouth's line: `smoothstep(0, 0, x)` is a
step. Seen from close, that cut of a few metres crossed the frame.

**The fix:** behind its mouth a plume now fades over two half widths and 2 m at least
(`RIVER_PLUME_BEHIND` in `shaders/water.slang`); where the channel runs straight it fades as
before.

**Found on the way:** the lake was traced twice. It is two pockets deeper than the lakes' 0.5 m
threshold in one sheet of water, and each pocket's flood took the whole sheet. Its water was
drawn twice, and the outlet's trim cut one copy only. `forge_procgen::lake_waters` now keeps a
sheet once, as deep as its deepest pocket: the island has 3 lakes, not 4. Its outlet's sill
changes the ground a little, so the rays' simplified ground is cut at another error (0.692 →
0.667 m).
