# #109: the rivers' grading to the sea (2026-10-01)

The island before (left, 27b53ec) and with the channels carrying the hillslopes' material away
(right, `city-blocks --island 7 --island-channel-ha 25`), the waves held at frame 60. The rule
is in the code and off by default (D-040): the island you run is still the left one.

- `grading-far.png`: the first view, the island from 2.5 km (`--view -6500,2500,-1416,-90,-35`),
  the south beach from 60 m (`--view 0,60,5160,0,-60`).
- `grading-rivers.png`: each build's own logged view halfway down its largest river, from 3 m
  and from 60 m up. The two are not the same place: the network changes with the field (43
  rivers before, 47 after), and the halfway point moved from a plain to a valley.
- `grading-cross.png`: the same camera, 3 m over the plain's river before (left) and on the
  graded island (right): the two logged views are 12 m apart in x and 65 m in z, but the
  ground under them is 152 m lower on the right (236.8 → 84.3 m). The river's whole profile
  dropped to the stream power's while the ridges stayed, so the valleys are canyons.
- `grading-mouths.png`: each build's own logged largest mouth (`into_sea`) and gentlest mouth
  (`gentle_sea`), from 4 m over the water and from 60 m up.

## What changed

- The erosion's diffusion sweep never raises a cell draining `channel_area` or more, and raises
  a smaller channel's cell by the share of its catchment short of that
  (`ErosionParams::channel_area`, 0 and off by default; `genesis --channel-ha`, `city-blocks
  --island-channel-ha`).
- The demo logs every mouth's fall over its last 160 m (`the rivers' last 160 m to the sea`).

## Numbers

| | before | 25 ha | 100 ha | 400 ha |
|---|---|---|---|---|
| mouths | 25 | 27 | 26 | 24 |
| over 5 % / over 10 % | 25 / 21 | 1 / 0 | 8 / 3 | 18 / 14 |
| the 14 m river's fall | 15 % | 4 % | 4 % | 4 % |
| the 4 m rivers' falls | 15–24 % | 3–5 % | 7–12 % | 13–20 % |
| lakes of a hectare or more | 14 | 0 | 3 | 5 |
| the water most under its banks | 3.9 m | 12.4 m | 12.3 m | 10.1 m |
| points over 2 m under (of 16–17 k) | 621 | 6 314 | 6 213 | 5 147 |
| the heights | −60 to 534 m | the same | the same | the same |

The worst stretch of the largest river at 25 ha: its level holds at 167.3 m over 40 m while
the ground along its smoothed course rises 7 m and comes back, a wall the course climbs at a
D8 corner of a floor that is now a slot one cell wide.

## For the owner's eye

- The mouths: a river running into the surf instead of a cascade, and no white water down the
  last reach.
- The valleys: canyons up to 150 m deeper, their floor a slot a cell wide; the ribbon leaves it
  at the bends and the carve cuts the wall. The fill the rule removes is in effect the island's
  alluvium, 150 steps of it, and the valleys want it back as *transported* sediment
  (deposited where the river can no longer carry it) rather than removed: D-040.
- The lakes are gone on the right: they were dams of the fill.

## Checks

- The batch against abc95bc's build (the images of 27b53ec, whose change was only the flags):
  every island and water image at 0 px (the default island is unchanged: `channel_area` 0 is
  the old arithmetic exactly); the A/B harness and mesh against fallback at 0 px.

## Later the same morning: the coastal plain

The steep mouths went with a coastal plain in the uplift (D-041, `IslandParams::plain`), not
with this rule: the diffusion's fill is as large as the valley walls are steep, and the hills
rose straight out of the sea. With the plain every mouth falls 1–5 % and the island keeps 11
lakes; `docs/demos/island.md`, "The coastal plain", and `reports/2026-10-01-112/`. Only the asteroids' ballad at frame 600 differs (361 and 390 px, ꟻLIP mean
  0.0014), and the new build captured twice differs from itself by as much (0.0015): #71's
  flake.
- Validation is not re-run: no shader, pass or resource changes.
- Tests (a unit test of the sweep on a V valley), clippy, fmt.
