# #108: the sea's water, to choose (2026-10-07)

The sea's colour under the water and from above comes from two sets of numbers: how fast its water
takes each channel away (its absorption) and the light it scatters back (its colour seen straight
down). `city-blocks --island 7 --sea-water NAME` draws the sea with one of six
(`forge_render::SeaWater::NAMED`); `teal` is the default, unchanged. Frame 60 with the fixed step,
1600 × 900, seed 7.

| name | absorption, m⁻¹ (r, g, b) | 1/e in green, blue | scattered back (r, g, b) |
|---|---|---|---|
| `teal` (now) | 0.35, 0.07, 0.05 | 14 m, 20 m | 0.003, 0.013, 0.016 |
| `clear` | 0.30, 0.045, 0.032 | 22 m, 31 m | 0.003, 0.013, 0.016 |
| `blue` | 0.35, 0.07, 0.035 | 14 m, 29 m | 0.0015, 0.008, 0.018 |
| `clear-blue` | 0.32, 0.05, 0.022 | 20 m, 45 m | 0.0015, 0.008, 0.018 |
| `turquoise` | 0.30, 0.04, 0.03 | 25 m, 33 m | 0.002, 0.016, 0.019 |
| `ocean` | 0.34, 0.06, 0.017 | 17 m, 59 m | 0.0007, 0.004, 0.017 |

- `clear` keeps the teal's colour and sees half again as far.
- `blue` is as clear as the teal, glowing blue.
- `clear-blue` is both.
- `turquoise` is clearer still and brighter, green and blue: a lagoon's.
- `ocean` is close to pure sea water, which takes the blue least and scatters it most: the open
  ocean's deep blue.

`under.png`, a row per water in the table's order; the columns:
- across the floor towards the shore, 3 m under (`4770,-3.0,-2847,91.1,-10`);
- up at Snell's window from there (`…,40`);
- towards the sun (`4770,-3.0,-2847,-60,35`);
- the floor from 6 m (`4770,-6.0,-2847,91.1,-30`).

`above.png`, a row per water; the columns:
- the shallows off the beach from 25 m (`0,25,5214,0,-20`): the sand shows through a metre or two
  whichever the water, so they change little;
- the coast from 150 m (`0,150,5500,0,-25`): the deep water past the shallows takes the water's
  own colour, from teal to the ocean's blue.

The exposure is the frame's, as from above; one that adapts under the water and light shafts are
the other levers. The lakes keep their dark water (`LAKE_ABSORPTION`), a choice of its own.
